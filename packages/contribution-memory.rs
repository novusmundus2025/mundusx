//! OS-independent media eligibility, measured after applying the user's cap.
use serde::Serialize;

pub const POLICY: &str = include_str!("../workers/media/contribution-policy.json");
pub const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct MediaBudget {
    pub contribution_budget_bytes: Option<u64>,
    pub minimum_media_budget_bytes: u64,
    pub cap_percent: u8,
    pub eligible: bool,
}

impl MediaBudget {
    pub fn new(total_bytes: Option<u64>, cap_percent: u8) -> Self {
        let policy: serde_json::Value =
            serde_json::from_str(POLICY).expect("bundled contribution policy");
        let minimum = policy["minimum_media_budget_bytes"].as_u64().unwrap();
        let maximum = policy["maximum_cap_percent"].as_u64().unwrap();
        let budget = total_bytes
            .filter(|total| *total > 0)
            .filter(|_| cap_percent > 0 && u64::from(cap_percent) <= maximum)
            .map(|total| ((u128::from(total) * u128::from(cap_percent)) / 100) as u64);
        Self {
            contribution_budget_bytes: budget,
            minimum_media_budget_bytes: minimum,
            cap_percent,
            eligible: budget.is_some_and(|bytes| bytes >= minimum),
        }
    }

    pub fn detect(cap: u8) -> Self {
        Self::new(physical_memory_bytes(), cap)
    }

    pub fn description(&self) -> String {
        let budget = self
            .contribution_budget_bytes
            .map(|bytes| format!("{:.2} GiB", bytes as f64 / GIB as f64))
            .unwrap_or_else(|| "unknown (memory detection failed or cap is unset/invalid)".into());
        format!("Image generation, image editing, video generation and image-to-video require at least {} GiB of contributed memory; current budget: {budget} after the {}% cap. Individual models may require more.",
            self.minimum_media_budget_bytes / GIB, self.cap_percent)
    }

    pub fn require(&self) -> Result<(), String> {
        if self.eligible {
            Ok(())
        } else {
            Err(self.description())
        }
    }
}

pub fn physical_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        return std::str::from_utf8(&output.stdout)
            .ok()?
            .trim()
            .parse()
            .ok();
    }
    #[cfg(target_os = "linux")]
    {
        let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = contents
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))?;
        return line
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()?
            .checked_mul(1024);
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        return (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys);
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eligibility_uses_capped_memory_without_rounding_up() {
        for (total, cap, eligible) in [
            (32, 50, false),
            (32, 74, false),
            (32, 75, true),
            (48, 50, true),
            (64, 30, false),
            (64, 40, true),
            (24, 80, false),
        ] {
            assert_eq!(
                MediaBudget::new(Some(total * GIB), cap).eligible,
                eligible,
                "{total} GiB at {cap}%"
            );
        }
        assert!(!MediaBudget::new(Some(32 * GIB - 1), 75).eligible);
        assert!(!MediaBudget::new(None, 80).eligible);
        assert!(!MediaBudget::new(Some(128 * GIB), 0).eligible);
        assert!(!MediaBudget::new(Some(128 * GIB), 81).eligible);
    }
}
