//! How fast colony hours pass in real time: pause and 1x / 2x / 3x.
//! Floats are fine here: this is presentation, the sim only counts ticks.

/// Real seconds per simulated hour at 1x speed (GDD: 1 hour = 5 s).
pub const SECONDS_PER_HOUR_AT_1X: f64 = 5.0;
/// Speeds the player can pick.
pub const SPEEDS: [u32; 3] = [1, 2, 3];

#[derive(Debug, Clone, PartialEq)]
pub struct Clock {
    pub paused: bool,
    speed: u32,
    /// Real seconds not yet turned into an hour.
    carry: f64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            paused: false,
            speed: 1,
            carry: 0.0,
        }
    }
}

impl Clock {
    pub fn speed(&self) -> u32 {
        self.speed
    }

    /// Ignores speeds the game doesn't offer.
    pub fn set_speed(&mut self, speed: u32) {
        if SPEEDS.contains(&speed) {
            self.speed = speed;
        }
    }

    /// Feed one frame's real seconds; returns how many hours to tick now.
    pub fn hours(&mut self, frame_seconds: f64) -> u32 {
        if self.paused {
            return 0;
        }
        self.carry += frame_seconds * f64::from(self.speed);
        let mut hours = 0;
        while self.carry >= SECONDS_PER_HOUR_AT_1X {
            self.carry -= SECONDS_PER_HOUR_AT_1X;
            hours += 1;
        }
        hours
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_types::HOURS_PER_SOL;

    /// Feed `seconds` in 60 fps frames.
    fn run(clock: &mut Clock, seconds: f64) -> u32 {
        let frames = (seconds * 60.0).round() as u32;
        (0..frames).map(|_| clock.hours(1.0 / 60.0)).sum()
    }

    #[test]
    fn one_hour_takes_five_seconds_at_1x() {
        let mut clock = Clock::default();
        assert_eq!(run(&mut clock, 4.9), 0);
        assert_eq!(run(&mut clock, 0.2), 1);
    }

    #[test]
    fn one_sol_takes_two_minutes_ten_at_1x() {
        let mut clock = Clock::default();
        assert_eq!(
            u64::from(run(&mut clock, 130.0 + 0.01)),
            HOURS_PER_SOL,
            "26 h x 5 s = 130 s"
        );
    }

    #[test]
    fn speeds_multiply_and_pause_stops() {
        for speed in SPEEDS {
            let mut clock = Clock::default();
            clock.set_speed(speed);
            assert_eq!(run(&mut clock, 10.01), 2 * speed, "{speed}x");
        }
        let mut clock = Clock {
            paused: true,
            ..Clock::default()
        };
        assert_eq!(run(&mut clock, 60.0), 0);
        clock.set_speed(4);
        assert_eq!(clock.speed(), 1, "no 4x");
    }
}
