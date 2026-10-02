use std::time::Instant;

const ALPHA: f64 = 0.3;
const MIN_RATE_FOR_ETA: f64 = 1024.0;

#[derive(Debug)]
pub struct SpeedMeter {
    last_bytes: u64,
    last_at: Instant,
    ema: f64,
}

impl SpeedMeter {
    pub fn new(bytes_now: u64) -> Self {
        Self {
            last_bytes: bytes_now,
            last_at: Instant::now(),
            ema: 0.0,
        }
    }

    pub fn reset(&mut self, bytes_now: u64) {
        *self = Self::new(bytes_now);
    }

    /// Feed the current cumulative byte count; returns smoothed bytes/sec.
    pub fn sample(&mut self, bytes_now: u64) -> f64 {
        let now = Instant::now();
        let dt = now.duration_since(self.last_at).as_secs_f64();
        if dt < 0.05 {
            return self.ema;
        }
        let delta = bytes_now.saturating_sub(self.last_bytes) as f64;
        let inst = delta / dt;
        self.ema = if self.ema == 0.0 {
            inst
        } else {
            ALPHA * inst + (1.0 - ALPHA) * self.ema
        };
        self.last_bytes = bytes_now;
        self.last_at = now;
        self.ema
    }

    pub fn rate(&self) -> f64 {
        self.ema
    }

    pub fn eta_secs(&self, remaining: u64) -> Option<u64> {
        if self.ema < MIN_RATE_FOR_ETA {
            return None;
        }
        Some((remaining as f64 / self.ema).ceil() as u64)
    }
}
