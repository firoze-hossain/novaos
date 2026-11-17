//! Prometheus Exposition Format Models and Metric Registry.
//!
//! Conforms to Prometheus text-based exposition format specification 0.0.4.
//! Encapsulates Gauges, Counters, and MetricFamilies with efficient serialization
//! directly into `ScratchBuffer`.

#![forbid(unsafe_code)]

use crate::buffer::ScratchBuffer;

/// Type of Prometheus metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    Counter,
    Gauge,
    Untyped,
}

impl MetricType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MetricType::Counter => "counter",
            MetricType::Gauge => "gauge",
            MetricType::Untyped => "untyped",
        }
    }
}

/// A single sample with labels and floating-point value.
#[derive(Debug, Clone)]
pub struct MetricSample {
    pub labels: Vec<(String, String)>,
    pub value: f64,
    pub timestamp: Option<u64>,
}

/// A family of related metric samples sharing a common name, help documentation, and type.
#[derive(Debug, Clone)]
pub struct MetricFamily {
    pub name: String,
    pub help: String,
    pub metric_type: MetricType,
    pub samples: Vec<MetricSample>,
}

impl MetricFamily {
    pub fn new(name: impl Into<String>, help: impl Into<String>, metric_type: MetricType) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            metric_type,
            samples: Vec::new(),
        }
    }

    pub fn counter(name: impl Into<String>, help: impl Into<String>) -> Self {
        Self::new(name, help, MetricType::Counter)
    }

    pub fn gauge(name: impl Into<String>, help: impl Into<String>) -> Self {
        Self::new(name, help, MetricType::Gauge)
    }

    pub fn add_sample(&mut self, labels: &[(&str, &str)], value: f64) -> &mut Self {
        let label_vec = labels
            .iter()
            .map(|&(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.samples.push(MetricSample {
            labels: label_vec,
            value,
            timestamp: None,
        });
        self
    }

    pub fn add_scalar(&mut self, value: f64) -> &mut Self {
        self.samples.push(MetricSample {
            labels: Vec::new(),
            value,
            timestamp: None,
        });
        self
    }

    /// Serializes this metric family directly into the scratch buffer.
    pub fn render(&self, buffer: &mut ScratchBuffer) -> bool {
        if self.samples.is_empty() {
            return true;
        }

        if !buffer.write_help(&self.name, &self.help) {
            return false;
        }
        if !buffer.write_type(&self.name, self.metric_type.as_str()) {
            return false;
        }

        for sample in &self.samples {
            // Convert to slice of references for zero-allocation formatting
            let labels: Vec<(&str, &str)> = sample
                .labels
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            if !buffer.write_metric(&self.name, &labels, sample.value, sample.timestamp) {
                return false;
            }
        }

        buffer.write_byte(b'\n')
    }
}

/// Registry that aggregates metric families and coordinates exposition rendering.
#[derive(Debug, Default, Clone)]
pub struct MetricsRegistry {
    families: Vec<MetricFamily>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            families: Vec::new(),
        }
    }

    pub fn add_family(&mut self, family: MetricFamily) {
        self.families.push(family);
    }

    pub fn len(&self) -> usize {
        self.families.len()
    }

    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    pub fn clear(&mut self) {
        self.families.clear();
    }

    pub fn families(&self) -> &[MetricFamily] {
        &self.families
    }

    /// Renders all registered metrics into a reusable `ScratchBuffer` in Prometheus 0.0.4 text format.
    pub fn render(&self, buffer: &mut ScratchBuffer) -> bool {
        buffer.clear();
        for family in &self.families {
            if !family.render(buffer) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metric_family_rendering() {
        let mut mf = MetricFamily::counter(
            "node_network_receive_bytes_total",
            "Network device receive bytes",
        );
        mf.add_sample(&[("device", "eth0")], 10485760.0);
        mf.add_sample(&[("device", "lo")], 2048.0);

        let mut buf = ScratchBuffer::new();
        assert!(mf.render(&mut buf));

        let out = buf.as_str();
        assert!(out.contains("# HELP node_network_receive_bytes_total Network device receive bytes\n"));
        assert!(out.contains("# TYPE node_network_receive_bytes_total counter\n"));
        assert!(out.contains("node_network_receive_bytes_total{device=\"eth0\"} 10485760.0\n"));
        assert!(out.contains("node_network_receive_bytes_total{device=\"lo\"} 2048.0\n"));
    }

    #[test]
    fn test_registry_rendering() {
        let mut reg = MetricsRegistry::new();
        let mut cpu = MetricFamily::gauge("node_cpu_utilization_ratio", "CPU utilization 0.0 to 1.0");
        cpu.add_scalar(0.245);
        reg.add_family(cpu);

        let mut buf = ScratchBuffer::new();
        assert!(reg.render(&mut buf));

        let out = buf.as_str();
        assert!(out.contains("node_cpu_utilization_ratio 0.245\n"));
    }
}
