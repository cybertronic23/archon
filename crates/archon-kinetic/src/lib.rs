//! Chronos: convert sparse waypoints into high-rate joint commands.

use archon_embodied::{now_us, ActionProposal, JointCommand, JointWaypoint};

/// Interpolates sparse waypoints to a fixed control rate (Hz).
#[derive(Debug, Clone)]
pub struct Chronos {
    pub rate_hz: f64,
    pub joint_names: Vec<String>,
}

impl Chronos {
    pub fn new(rate_hz: f64, joint_names: Vec<String>) -> Self {
        Self {
            rate_hz: rate_hz.max(1.0),
            joint_names,
        }
    }

    /// Desktop-arm defaults: 6 DoF @ 50 Hz.
    pub fn desktop_arm_6dof() -> Self {
        Self::new(
            50.0,
            (1..=6).map(|i| format!("joint_{i}")).collect(),
        )
    }

    pub fn interpolate_proposal(&self, proposal: &ActionProposal) -> Vec<JointCommand> {
        self.interpolate_waypoints(&proposal.waypoints)
    }

    pub fn interpolate_waypoints(&self, waypoints: &[JointWaypoint]) -> Vec<JointCommand> {
        if waypoints.is_empty() {
            return Vec::new();
        }

        let mut sorted = waypoints.to_vec();
        sorted.sort_by(|a, b| {
            a.t_sec
                .partial_cmp(&b.t_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Ensure we start at t=0 with first waypoint if needed.
        if sorted[0].t_sec > 0.0 {
            let mut first = sorted[0].clone();
            first.t_sec = 0.0;
            sorted.insert(0, first);
        }

        let t_end = sorted.last().map(|w| w.t_sec).unwrap_or(0.0).max(0.0);
        let dt = 1.0 / self.rate_hz;
        let mut cmds = Vec::new();
        let mut t = 0.0;
        let base_stamp = now_us();

        while t <= t_end + 1e-9 {
            let (pos, grip) = sample_at(&sorted, t);
            let stamp_offset = (t * 1_000_000.0) as u64;
            cmds.push(JointCommand {
                stamp_us: base_stamp.saturating_add(stamp_offset),
                names: self.joint_names.clone(),
                positions: pos,
                gripper_open: grip,
            });
            t += dt;
        }

        cmds
    }
}

fn sample_at(waypoints: &[JointWaypoint], t: f64) -> (Vec<f64>, Option<f64>) {
    if waypoints.len() == 1 {
        return (
            waypoints[0].positions.clone(),
            waypoints[0].gripper_open,
        );
    }

    if t <= waypoints[0].t_sec {
        return (
            waypoints[0].positions.clone(),
            waypoints[0].gripper_open,
        );
    }

    let last = waypoints.last().unwrap();
    if t >= last.t_sec {
        return (last.positions.clone(), last.gripper_open);
    }

    for i in 0..waypoints.len() - 1 {
        let a = &waypoints[i];
        let b = &waypoints[i + 1];
        if t >= a.t_sec && t <= b.t_sec {
            let span = (b.t_sec - a.t_sec).max(1e-9);
            let alpha = (t - a.t_sec) / span;
            let n = a.positions.len().min(b.positions.len());
            let mut pos = Vec::with_capacity(n);
            for j in 0..n {
                pos.push(a.positions[j] + alpha * (b.positions[j] - a.positions[j]));
            }
            let grip = match (a.gripper_open, b.gripper_open) {
                (Some(ga), Some(gb)) => Some(ga + alpha * (gb - ga)),
                (Some(ga), None) => Some(ga),
                (None, Some(gb)) => Some(gb),
                (None, None) => None,
            };
            return (pos, grip);
        }
    }

    (last.positions.clone(), last.gripper_open)
}

/// Per-joint position limits (radians).
#[derive(Debug, Clone)]
pub struct JointLimits {
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
}

impl JointLimits {
    pub fn desktop_arm_6dof() -> Self {
        // Conservative symmetric limits for a small desktop arm.
        Self {
            lower: vec![-2.5; 6],
            upper: vec![2.5; 6],
        }
    }

    pub fn contains(&self, positions: &[f64]) -> Result<(), String> {
        for (i, &p) in positions.iter().enumerate() {
            let lo = self.lower.get(i).copied().unwrap_or(f64::NEG_INFINITY);
            let hi = self.upper.get(i).copied().unwrap_or(f64::INFINITY);
            if p < lo || p > hi {
                return Err(format!(
                    "joint[{i}]={p:.4} outside limits [{lo:.4}, {hi:.4}]"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::JointWaypoint;

    #[test]
    fn interpolates_two_waypoints() {
        let chronos = Chronos::new(10.0, vec!["j1".into()]);
        let wps = vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: vec![0.0],
                gripper_open: Some(1.0),
            },
            JointWaypoint {
                t_sec: 1.0,
                positions: vec![1.0],
                gripper_open: Some(0.0),
            },
        ];
        let cmds = chronos.interpolate_waypoints(&wps);
        assert!(cmds.len() >= 10);
        assert!((cmds[0].positions[0] - 0.0).abs() < 1e-6);
        assert!((cmds.last().unwrap().positions[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn limits_reject_oob() {
        let lim = JointLimits {
            lower: vec![-1.0],
            upper: vec![1.0],
        };
        assert!(lim.contains(&[0.5]).is_ok());
        assert!(lim.contains(&[1.5]).is_err());
    }
}
