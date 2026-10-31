//! CPU / RAM / GPU / network / disk sampling. Only runs while the Stats page is visible.

use std::collections::VecDeque;
use std::time::Instant;
use windows::core::w;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Performance::*;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

pub const HISTORY: usize = 60;

pub struct Stats {
    prev_cpu: Option<(u64, u64)>,
    prev_net: Option<(u64, u64, Instant)>,
    pdh: Option<(isize, isize)>,
    pdh_tried: bool,
    pub cpu: VecDeque<f32>,
    pub gpu: VecDeque<f32>,
    pub down: VecDeque<f64>,
    pub up: VecDeque<f64>,
    pub ram_used: f64,
    pub ram_total: f64,
    pub disk_free: f64,
    pub disk_total: f64,
    pub gpu_ok: bool,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    q.push_back(v);
    while q.len() > HISTORY {
        q.pop_front();
    }
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            prev_cpu: None,
            prev_net: None,
            pdh: None,
            pdh_tried: false,
            cpu: VecDeque::new(),
            gpu: VecDeque::new(),
            down: VecDeque::new(),
            up: VecDeque::new(),
            ram_used: 0.0,
            ram_total: 0.0,
            disk_free: 0.0,
            disk_total: 0.0,
            gpu_ok: false,
        }
    }

    pub fn sample(&mut self) {
        unsafe {
            // CPU
            let (mut idle, mut kern, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetSystemTimes(Some(&mut idle), Some(&mut kern), Some(&mut user)).is_ok() {
                let (i, t) = (ft(idle), ft(kern) + ft(user));
                if let Some((pi, pt)) = self.prev_cpu {
                    let dt = t.saturating_sub(pt).max(1);
                    let di = i.saturating_sub(pi);
                    push(&mut self.cpu, (1.0 - di as f32 / dt as f32).clamp(0.0, 1.0) * 100.0);
                }
                self.prev_cpu = Some((i, t));
            }
            // RAM
            let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if GlobalMemoryStatusEx(&mut m).is_ok() {
                self.ram_total = m.ullTotalPhys as f64;
                self.ram_used = (m.ullTotalPhys - m.ullAvailPhys) as f64;
            }
            // Disk (system drive)
            let (mut free, mut total) = (0u64, 0u64);
            if GetDiskFreeSpaceExW(w!("C:\\"), Some(&mut free), Some(&mut total), None).is_ok() {
                self.disk_free = free as f64;
                self.disk_total = total as f64;
            }
            // Network: physical interfaces that are up.
            let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut table).is_ok() && !table.is_null() {
                let n = (*table).NumEntries as usize;
                let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
                let (mut rx, mut tx) = (0u64, 0u64);
                for r in rows {
                    let hardware = r.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
                    if hardware && r.OperStatus.0 == 1 && r.Type != 24 {
                        rx += r.InOctets;
                        tx += r.OutOctets;
                    }
                }
                FreeMibTable(table as *const _);
                let now = Instant::now();
                if let Some((prx, ptx, pt)) = self.prev_net {
                    let dt = now.duration_since(pt).as_secs_f64().max(0.05);
                    self.down.push_back(rx.saturating_sub(prx) as f64 / dt);
                    self.up.push_back(tx.saturating_sub(ptx) as f64 / dt);
                    while self.down.len() > HISTORY {
                        self.down.pop_front();
                    }
                    while self.up.len() > HISTORY {
                        self.up.pop_front();
                    }
                }
                self.prev_net = Some((rx, tx, now));
            }
            // GPU (3D engine utilisation summed over processes)
            if !self.pdh_tried {
                self.pdh_tried = true;
                let mut q = 0isize;
                let mut c = 0isize;
                if PdhOpenQueryW(None, 0, &mut q) == 0 {
                    if PdhAddEnglishCounterW(q, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut c) == 0 {
                        PdhCollectQueryData(q);
                        self.pdh = Some((q, c));
                    } else {
                        PdhCloseQuery(q);
                    }
                }
            }
            if let Some((q, c)) = self.pdh {
                if PdhCollectQueryData(q) == 0 {
                    let (mut size, mut count) = (0u32, 0u32);
                    PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, None);
                    if size > 0 {
                        let n = size as usize / std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() + 1;
                        let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![std::mem::zeroed(); n];
                        if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(buf.as_mut_ptr())) == 0 {
                            let total: f64 = buf[..count as usize].iter().map(|i| i.FmtValue.Anonymous.doubleValue).sum();
                            push(&mut self.gpu, total.clamp(0.0, 100.0) as f32);
                            self.gpu_ok = true;
                        }
                    }
                }
            }
        }
    }
}

pub fn bytes(b: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", units[i])
    } else if v < 10.0 {
        format!("{v:.1} {}", units[i])
    } else {
        format!("{v:.0} {}", units[i])
    }
}
