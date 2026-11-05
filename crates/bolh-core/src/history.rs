//! История версий: автоматические снимки файла и их прореживание.
//!
//! Снимки лежат внутри пространства, в `.bolh/history/<id файла>/`, поэтому
//! переезжают вместе с папкой и синхронизируются облаком. Папка названа по
//! id из файла, а не по имени: переименование не рвёт историю.
//!
//! Сколько хранить (раздел 3 спецификации): все снимки за последние сутки,
//! потом по одному в день за месяц, потом по одному в неделю.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDateTime, Utc};
use uuid::Uuid;

use crate::error::{Result, io};
use crate::format::FileKind;
use crate::fsutil::write_atomic;

/// Не чаще одного автоматического снимка за этот промежуток. Иначе
/// автосохранение на каждый символ дало бы тысячи почти одинаковых снимков.
pub const MIN_INTERVAL_MINUTES: i64 = 5;
const STAMP: &str = "%Y%m%dT%H%M%S%.3fZ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub at: DateTime<Utc>,
    pub path: PathBuf,
}

pub struct History {
    dir: PathBuf,
}

impl History {
    pub fn new(dir: PathBuf) -> Self {
        History { dir }
    }

    fn dir_for(&self, id: Uuid) -> PathBuf {
        self.dir.join(id.to_string())
    }

    /// Все снимки файла, от новых к старым.
    pub fn list(&self, id: Uuid) -> Result<Vec<Snapshot>> {
        let dir = self.dir_for(id);
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(&dir)(e)),
        };
        let mut out: Vec<Snapshot> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let at = parse_stamp(path.file_stem()?.to_str()?)?;
                FileKind::from_path(&path)?;
                Some(Snapshot { at, path })
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.at));
        Ok(out)
    }

    pub fn read(&self, snap: &Snapshot) -> Result<Vec<u8>> {
        fs::read(&snap.path).map_err(io(&snap.path))
    }

    /// Делает снимок, если он нужен, и прореживает старые.
    ///
    /// Без `force` снимок не делается, когда содержимое не изменилось или
    /// прошлый снимок моложе [`MIN_INTERVAL_MINUTES`]. С `force` — делается
    /// всегда, если содержимое отличается: так перед откатом сохраняется
    /// текущее состояние, и сам откат тоже можно отменить.
    pub fn record(
        &self,
        id: Uuid,
        kind: FileKind,
        bytes: &[u8],
        now: DateTime<Utc>,
        tz: FixedOffset,
        force: bool,
    ) -> Result<Option<Snapshot>> {
        let existing = self.list(id)?;
        if let Some(last) = existing.first() {
            let same = fs::read(&last.path).map(|b| b == bytes).unwrap_or(false);
            let recent = now - last.at < Duration::minutes(MIN_INTERVAL_MINUTES);
            if same || (recent && !force) {
                return Ok(None);
            }
        }
        // Два снимка в одну миллисекунду (или часы ушли назад) не должны
        // затереть друг друга.
        let mut at = now;
        while existing.iter().any(|s| s.at == trunc_ms(at)) {
            at += Duration::milliseconds(1);
        }
        let at = trunc_ms(at);
        let path = self
            .dir_for(id)
            .join(format!("{}.{}", at.format(STAMP), kind.ext()));
        write_atomic(&path, bytes)?;
        self.prune(id, now, tz)?;
        Ok(Some(Snapshot { at, path }))
    }

    /// Удаляет снимки, которые не нужны по правилу хранения.
    pub fn prune(&self, id: Uuid, now: DateTime<Utc>, tz: FixedOffset) -> Result<()> {
        let snaps = self.list(id)?;
        let times: Vec<_> = snaps.iter().map(|s| s.at).collect();
        for i in plan_prune(&times, now, tz) {
            let p = &snaps[i].path;
            match fs::remove_file(p) {
                Ok(()) => {}
                // Уже удалил другой компьютер через облако — это нормально.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(p)(e)),
            }
        }
        Ok(())
    }

    /// Удаляет всю историю файла — когда файл удалён из корзины навсегда.
    pub fn remove_all(&self, id: Uuid) -> Result<()> {
        let dir = self.dir_for(id);
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(dir)(e)),
        }
    }
}

fn trunc_ms(t: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(t.timestamp_millis()).expect("время в допустимых пределах")
}

fn parse_stamp(s: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, STAMP)
        .ok()
        .map(|n| n.and_utc())
}

/// Какие снимки удалить (номера в `times`). Чистая функция, чтобы правило
/// хранения можно было проверить тестами без диска.
///
/// - моложе суток — оставить все;
/// - от суток до 30 дней — оставить самый поздний за каждый день;
/// - старше 30 дней — самый поздний за каждую неделю.
///
/// Дни и недели считаются по местному времени (`tz`): «один снимок в день»
/// должен совпадать с днём человека, а не с днём по Гринвичу. Границы
/// календарные, а не «24 часа от сейчас», поэтому однажды оставленный
/// снимок не выпадает при следующем прореживании.
pub fn plan_prune(times: &[DateTime<Utc>], now: DateTime<Utc>, tz: FixedOffset) -> Vec<usize> {
    #[derive(Hash, PartialEq, Eq)]
    enum Bucket {
        Day(i32, u32),
        Week(i32, u32),
    }
    let mut best: HashMap<Bucket, usize> = HashMap::new();
    let mut candidates = Vec::new();
    for (i, &t) in times.iter().enumerate() {
        let age = now - t;
        if age < Duration::days(1) {
            continue; // включая снимки «из будущего» от компьютера со сбитыми часами
        }
        let local = t.with_timezone(&tz);
        let bucket = if age < Duration::days(30) {
            Bucket::Day(local.year(), local.ordinal())
        } else {
            let w = local.iso_week();
            Bucket::Week(w.year(), w.week())
        };
        candidates.push(i);
        best.entry(bucket)
            .and_modify(|b| {
                if times[i] > times[*b] {
                    *b = i;
                }
            })
            .or_insert(i);
    }
    let keep: Vec<usize> = best.into_values().collect();
    let mut out: Vec<usize> = candidates
        .into_iter()
        .filter(|i| !keep.contains(i))
        .collect();
    out.sort_unstable();
    out
}

/// Папка истории внутри пространства.
pub fn history_dir(meta_dir: &Path) -> PathBuf {
    meta_dir.join("history")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn t(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn kept(times: &[DateTime<Utc>], now: DateTime<Utc>, tz: FixedOffset) -> Vec<DateTime<Utc>> {
        let del = plan_prune(times, now, tz);
        times
            .iter()
            .enumerate()
            .filter(|(i, _)| !del.contains(i))
            .map(|(_, t)| *t)
            .collect()
    }

    #[test]
    fn keeps_everything_from_last_day() {
        let now = t(2026, 11, 5, 12, 0);
        let times: Vec<_> = (0..24 * 12)
            .map(|i| now - Duration::minutes(5 * i))
            .collect();
        assert!(plan_prune(&times, now, utc()).is_empty());
    }

    #[test]
    fn one_per_day_in_month_latest_of_day() {
        let now = t(2026, 11, 20, 12, 0);
        let times = vec![
            t(2026, 11, 10, 9, 0),
            t(2026, 11, 10, 18, 0),
            t(2026, 11, 10, 23, 59),
            t(2026, 11, 11, 0, 1),
        ];
        assert_eq!(
            kept(&times, now, utc()),
            vec![t(2026, 11, 10, 23, 59), t(2026, 11, 11, 0, 1)]
        );
    }

    #[test]
    fn one_per_week_after_month() {
        let now = t(2026, 11, 20, 12, 0);
        // 2026-08-03 — понедельник; неделя по ISO с понедельника.
        let times = vec![
            t(2026, 8, 3, 10, 0),
            t(2026, 8, 6, 10, 0),
            t(2026, 8, 9, 22, 0), // воскресенье той же недели — самый поздний
            t(2026, 8, 10, 1, 0), // понедельник следующей
        ];
        assert_eq!(
            kept(&times, now, utc()),
            vec![t(2026, 8, 9, 22, 0), t(2026, 8, 10, 1, 0)]
        );
    }

    #[test]
    fn days_follow_local_time() {
        let msk = FixedOffset::east_opt(3 * 3600).unwrap();
        let now = t(2026, 11, 20, 12, 0);
        // 22:30 и 21:30 по Гринвичу — это 01:30 и 00:30 следующего дня в Москве,
        // а 20:00 — 23:00 предыдущего: два разных московских дня.
        let times = vec![
            t(2026, 11, 10, 20, 0),
            t(2026, 11, 10, 21, 30),
            t(2026, 11, 10, 22, 30),
        ];
        assert_eq!(
            kept(&times, now, msk),
            vec![t(2026, 11, 10, 20, 0), t(2026, 11, 10, 22, 30)]
        );
        assert_eq!(kept(&times, now, utc()), vec![t(2026, 11, 10, 22, 30)]);
    }

    #[test]
    fn pruning_is_stable_over_time() {
        // Прореживание раз в час в течение 60 дней: в любой момент после
        // прохода выполняется правило, а самый свежий снимок никогда не теряется.
        let start = t(2026, 1, 1, 0, 0);
        let mut times: Vec<DateTime<Utc>> = Vec::new();
        for hour in 0..(60 * 24) {
            let now = start + Duration::hours(hour);
            times.push(now);
            let del = plan_prune(&times, now, utc());
            times = times
                .iter()
                .enumerate()
                .filter(|(i, _)| !del.contains(i))
                .map(|(_, t)| *t)
                .collect();
            assert_eq!(*times.last().unwrap(), now);
            // Повторный проход ничего не удаляет.
            assert!(plan_prune(&times, now, utc()).is_empty());
        }
        let now = start + Duration::hours(60 * 24 - 1);
        let last_day = times
            .iter()
            .filter(|t| now - **t < Duration::days(1))
            .count();
        assert_eq!(last_day, 24);
        // За 29 полных дней до этого — по одному в день.
        let month = times
            .iter()
            .filter(|t| {
                let a = now - **t;
                a >= Duration::days(1) && a < Duration::days(30)
            })
            .count();
        assert!((29..=30).contains(&month), "в месяце {month}");
        let older = times
            .iter()
            .filter(|t| now - **t >= Duration::days(30))
            .count();
        assert!((4..=6).contains(&older), "старше месяца {older}");
    }

    #[test]
    fn future_snapshots_are_kept() {
        let now = t(2026, 11, 5, 12, 0);
        let times = vec![now + Duration::days(3), now + Duration::days(3)];
        assert!(plan_prune(&times, now, utc()).is_empty());
    }

    #[test]
    fn record_respects_interval_force_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path().to_path_buf());
        let id = Uuid::new_v4();
        let k = FileKind::Document;
        let now = t(2026, 11, 5, 12, 0);
        assert!(h.record(id, k, b"a", now, utc(), false).unwrap().is_some());
        // То же содержимое — не нужен.
        assert!(
            h.record(id, k, b"a", now + Duration::hours(1), utc(), true)
                .unwrap()
                .is_none()
        );
        // Слишком рано.
        let soon = now + Duration::minutes(2);
        assert!(h.record(id, k, b"b", soon, utc(), false).unwrap().is_none());
        // С force — можно, даже в ту же миллисекунду.
        assert!(h.record(id, k, b"b", now, utc(), true).unwrap().is_some());
        let later = now + Duration::minutes(10);
        assert!(
            h.record(id, k, b"c", later, utc(), false)
                .unwrap()
                .is_some()
        );
        let list = h.list(id).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(h.read(&list[0]).unwrap(), b"c");
        assert!(list.windows(2).all(|w| w[0].at > w[1].at));
    }

    #[test]
    fn record_prunes_old() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path().to_path_buf());
        let id = Uuid::new_v4();
        let base = t(2026, 10, 1, 8, 0);
        for i in 0..3 {
            let at = base + Duration::hours(i);
            h.record(
                id,
                FileKind::Canvas,
                format!("{i}").as_bytes(),
                at,
                utc(),
                false,
            )
            .unwrap();
        }
        let now = t(2026, 10, 10, 8, 0);
        h.record(id, FileKind::Canvas, b"new", now, utc(), false)
            .unwrap();
        let list = h.list(id).unwrap();
        assert_eq!(list.len(), 2, "от 1 октября остался один снимок");
        assert_eq!(h.read(&list[1]).unwrap(), b"2");
    }

    #[test]
    fn unrelated_files_ignored_and_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path().to_path_buf());
        let id = Uuid::new_v4();
        assert!(h.list(id).unwrap().is_empty());
        fs::create_dir_all(dir.path().join(id.to_string())).unwrap();
        fs::write(dir.path().join(id.to_string()).join("desktop.ini"), "").unwrap();
        assert!(h.list(id).unwrap().is_empty());
        h.remove_all(id).unwrap();
        h.remove_all(id).unwrap();
    }
}
