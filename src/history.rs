//! Bounded plot history. Recent samples stay exact; older samples retain per-metric extrema.

use crate::{Metric, protocol, worker::TimedSample};
use std::collections::VecDeque;

const RAW_SECONDS: f64 = 60.0;
const MAX_BUCKETS: usize = 8192;

#[derive(Clone, Copy, Debug)]
pub struct PlotSample {
    pub seconds: f64,
    pub sample: protocol::Sample,
    pub amp_hours: f64,
    pub watt_hours: f64,
}

#[derive(Clone, Copy, Debug)]
struct Extremum {
    seconds: f64,
    value: f64,
}

impl Extremum {
    const INVALID: Self = Self {
        seconds: 0.0,
        value: f64::NAN,
    };
}

#[derive(Clone)]
struct Bucket {
    index: u64,
    first: PlotSample,
    last: PlotSample,
    lows: [Extremum; Metric::ALL.len()],
    highs: [Extremum; Metric::ALL.len()],
}

impl Bucket {
    fn new(index: u64, point: PlotSample) -> Self {
        let mut bucket = Self {
            index,
            first: point,
            last: point,
            lows: [Extremum::INVALID; Metric::ALL.len()],
            highs: [Extremum::INVALID; Metric::ALL.len()],
        };
        bucket.add(point);
        bucket
    }

    fn add(&mut self, point: PlotSample) {
        self.last = point;
        for metric in Metric::ALL {
            if let Some(value) = metric.value(&point) {
                let at = metric as usize;
                let extremum = Extremum {
                    seconds: point.seconds,
                    value,
                };
                if !self.lows[at].value.is_finite() || value < self.lows[at].value {
                    self.lows[at] = extremum;
                }
                if !self.highs[at].value.is_finite() || value > self.highs[at].value {
                    self.highs[at] = extremum;
                }
            }
        }
    }

    fn merge(&mut self, later: Self) {
        self.last = later.last;
        for at in 0..Metric::ALL.len() {
            if later.lows[at].value.is_finite()
                && (!self.lows[at].value.is_finite() || later.lows[at].value < self.lows[at].value)
            {
                self.lows[at] = later.lows[at];
            }
            if later.highs[at].value.is_finite()
                && (!self.highs[at].value.is_finite()
                    || later.highs[at].value > self.highs[at].value)
            {
                self.highs[at] = later.highs[at];
            }
        }
    }
}

#[derive(Clone)]
pub struct History {
    raw: VecDeque<PlotSample>,
    buckets: VecDeque<Bucket>,
    bucket_seconds: u64,
    origin: Option<f64>,
    amp_hours: f64,
    watt_hours: f64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            raw: VecDeque::new(),
            buckets: VecDeque::new(),
            bucket_seconds: 1,
            origin: None,
            amp_hours: 0.0,
            watt_hours: 0.0,
        }
    }
}

impl History {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn origin(&self) -> Option<f64> {
        self.origin
    }

    pub fn latest(&self) -> Option<&PlotSample> {
        self.raw.back()
    }

    pub fn totals(&self) -> (f64, f64) {
        (self.amp_hours, self.watt_hours)
    }

    pub fn raw(&self) -> &VecDeque<PlotSample> {
        &self.raw
    }

    pub fn push(&mut self, incoming: TimedSample, rate: u16) {
        if rate == 0 {
            return;
        }
        let origin = *self.origin.get_or_insert(incoming.seconds);
        let current = incoming.sample.current as f64;
        let voltage = incoming.sample.voltage as f64;
        // DLL Tps() returns 250,100,20,5,2,1 ms for 4..1000 SPS.
        let hours_per_sample = 1.0 / (3600.0 * f64::from(rate));
        self.amp_hours += current * hours_per_sample;
        self.watt_hours += voltage * current * hours_per_sample;
        let point = PlotSample {
            seconds: incoming.seconds,
            sample: incoming.sample,
            amp_hours: self.amp_hours,
            watt_hours: self.watt_hours,
        };
        self.raw.push_back(point);
        while self
            .raw
            .front()
            .is_some_and(|first| first.seconds < point.seconds - RAW_SECONDS)
        {
            self.raw.pop_front();
        }

        let index = ((point.seconds - origin).max(0.0) / self.bucket_seconds as f64) as u64;
        if let Some(last) = self.buckets.back_mut().filter(|last| last.index == index) {
            last.add(point);
        } else {
            self.buckets.push_back(Bucket::new(index, point));
        }
        while self.buckets.len() > MAX_BUCKETS {
            self.coarsen();
        }
    }

    fn coarsen(&mut self) {
        let mut merged: VecDeque<Bucket> = VecDeque::new();
        for mut bucket in self.buckets.drain(..) {
            bucket.index /= 2;
            if let Some(last) = merged.back_mut().filter(|last| last.index == bucket.index) {
                last.merge(bucket);
            } else {
                merged.push_back(bucket);
            }
        }
        self.buckets = merged;
        self.bucket_seconds *= 2;
    }

    pub fn chart_points(&self, metric: Metric, start: f64, end: f64) -> Vec<[f64; 2]> {
        let Some(origin) = self.origin else {
            return Vec::new();
        };
        // The exact buffer covers short, live views. Paused views use the archived buckets.
        if end - start <= RAW_SECONDS && self.raw.front().is_some_and(|first| first.seconds <= end)
        {
            let first = self.raw.partition_point(|point| point.seconds < start);
            let last = self.raw.partition_point(|point| point.seconds <= end);
            let count = last.saturating_sub(first);
            let width = count.div_ceil(1000).max(1);
            let mut points = Vec::with_capacity(count.min(2002));
            if let Some(point) = self.raw.get(first).filter(|_| first < last) {
                append_sample(&mut points, metric, point, origin);
            }
            for index in (first..last).step_by(width) {
                let mut low = Extremum::INVALID;
                let mut high = Extremum::INVALID;
                for point in self.raw.range(index..(index + width).min(last)) {
                    if let Some(value) = metric.value(point) {
                        update_extrema(&mut low, &mut high, point.seconds, value);
                    }
                }
                append_extrema(&mut points, low, high, origin);
            }
            if let Some(point) = self.raw.get(last.saturating_sub(1)) {
                append_sample(&mut points, metric, point, origin);
            }
            return points;
        }

        let selected: Vec<_> = self
            .buckets
            .iter()
            .filter(|bucket| bucket.last.seconds >= start && bucket.first.seconds <= end)
            .collect();
        let width = selected.len().div_ceil(1000).max(1);
        let mut points = Vec::with_capacity(selected.len().min(1000) * 2 + 2);
        if let Some(first) = selected.first() {
            append_sample(&mut points, metric, &first.first, origin);
        }
        for group in selected.chunks(width) {
            let mut low = Extremum::INVALID;
            let mut high = Extremum::INVALID;
            let at = metric as usize;
            for bucket in group {
                let lo = bucket.lows[at];
                let hi = bucket.highs[at];
                if lo.value.is_finite() {
                    update_extrema(&mut low, &mut high, lo.seconds, lo.value);
                }
                if hi.value.is_finite() {
                    update_extrema(&mut low, &mut high, hi.seconds, hi.value);
                }
            }
            append_extrema(&mut points, low, high, origin);
        }
        if let Some(last) = selected.last() {
            append_sample(&mut points, metric, &last.last, origin);
        }
        points
    }

    pub fn closest(&self, seconds: f64, short_window: bool) -> Option<PlotSample> {
        if short_window
            && self
                .raw
                .front()
                .is_some_and(|first| first.seconds <= seconds)
        {
            let next = self.raw.partition_point(|point| point.seconds < seconds);
            return match (next.checked_sub(1), self.raw.get(next)) {
                (Some(previous), Some(after)) => {
                    let before = self.raw[previous];
                    Some(
                        if (seconds - before.seconds).abs() <= (after.seconds - seconds).abs() {
                            before
                        } else {
                            *after
                        },
                    )
                }
                (Some(previous), None) => self.raw.get(previous).copied(),
                (None, after) => after.copied(),
            };
        }
        let next = self
            .buckets
            .partition_point(|bucket| bucket.last.seconds < seconds);
        let nearby = next.saturating_sub(1)..(next + 1).min(self.buckets.len());
        self.buckets
            .range(nearby)
            .flat_map(|bucket| [bucket.first, bucket.last])
            .min_by(|a, b| {
                (a.seconds - seconds)
                    .abs()
                    .total_cmp(&(b.seconds - seconds).abs())
            })
    }
}

fn update_extrema(low: &mut Extremum, high: &mut Extremum, seconds: f64, value: f64) {
    let point = Extremum { seconds, value };
    if !low.value.is_finite() || value < low.value {
        *low = point;
    }
    if !high.value.is_finite() || value > high.value {
        *high = point;
    }
}

fn append_extrema(points: &mut Vec<[f64; 2]>, low: Extremum, high: Extremum, origin: f64) {
    if !low.value.is_finite() {
        return;
    }
    let (first, second) = if low.seconds <= high.seconds {
        (low, high)
    } else {
        (high, low)
    };
    append_point(points, first, origin);
    append_point(points, second, origin);
}

fn append_sample(points: &mut Vec<[f64; 2]>, metric: Metric, sample: &PlotSample, origin: f64) {
    if let Some(value) = metric.value(sample) {
        append_point(
            points,
            Extremum {
                seconds: sample.seconds,
                value,
            },
            origin,
        );
    }
}

fn append_point(points: &mut Vec<[f64; 2]>, point: Extremum, origin: f64) {
    let time = point.seconds - origin;
    if points.last().is_none_or(|last| last[0] < time) {
        points.push([time, point.value]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seconds: f64, voltage: f32, current: f32) -> TimedSample {
        TimedSample {
            seconds,
            sample: protocol::Sample {
                voltage,
                current,
                dp: 0.0,
                dm: 0.0,
            },
        }
    }

    #[test]
    fn accumulated_charge_and_energy_reset_with_history() {
        let mut history = History::default();
        for second in 0..10 {
            history.push(sample(second as f64, 5.0, 2.0), 10);
        }
        let (ah, wh) = history.totals();
        assert!((ah - 2.0 / 3600.0).abs() < 1e-12);
        assert!((wh - 10.0 / 3600.0).abs() < 1e-12);
        history.clear();
        assert_eq!(history.totals(), (0.0, 0.0));
        assert!(history.latest().is_none());
    }

    #[test]
    fn archive_bounds_memory_and_keeps_early_extremes() {
        let mut history = History::default();
        for second in 0..12_000 {
            let voltage = if second == 100 { 12.0 } else { 5.0 };
            history.push(sample(second as f64, voltage, 1.0), 1);
        }
        assert!(history.buckets.len() <= MAX_BUCKETS);
        assert!(history.raw.len() <= 61);
        assert_eq!(history.origin(), Some(0.0));
        let all = history.chart_points(Metric::Voltage, 0.0, 12_000.0);
        assert_eq!(all.first().unwrap()[0], 0.0);
        assert_eq!(all.last().unwrap()[0], 11_999.0);
        assert!(all.len() <= 2002);
        assert!(all.iter().any(|point| point[1] == 12.0));
        assert!(
            history
                .chart_points(Metric::Voltage, 8400.0, 12_000.0)
                .first()
                .unwrap()[0]
                >= 8400.0
        );
        assert_eq!(history.closest(100.0, false).unwrap().seconds, 100.0);
    }

    #[test]
    fn thousand_sps_retains_one_minute_of_exact_samples() {
        let mut history = History::default();
        for index in 0..120_000 {
            history.push(sample(index as f64 / 1000.0, 5.0, 1.0), 1000);
        }
        assert!(history.raw.len() <= 60_001);
        assert!(history.buckets.len() <= 120);
        assert!(std::mem::size_of::<PlotSample>() <= 48);
        assert!(std::mem::size_of::<Bucket>() <= 400);
        assert!(history.chart_points(Metric::Current, 60.0, 120.0).len() <= 2002);
        assert!((history.totals().0 - 120.0 / 3600.0).abs() < 1e-10);
    }
}
