//! Language / primitive libraries for arm and planar car.

use anyhow::{bail, Result};
use archon_embodied::{
    now_us, ActionProposal, JointWaypoint, Policy, ResourceKind, WorldState,
};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotKind {
    Arm,
    DiffCar,
}

impl RobotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arm => "arm",
            Self::DiffCar => "diff_car",
        }
    }

    pub fn parse_model_hint(model: &str) -> Self {
        let m = model.to_ascii_lowercase();
        if m.contains("car") || m.contains("diff") || m.contains("mobile") {
            Self::DiffCar
        } else {
            Self::Arm
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionPrimitive {
    Home,
    Wave,
    Reach,
    Nod,
    OpenGripper,
    CloseGripper,
    Demo,
    // Diff car
    Forward,
    Backward,
    TurnLeft,
    TurnRight,
    Stop,
}

impl MotionPrimitive {
    pub fn id(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Wave => "wave",
            Self::Reach => "reach",
            Self::Nod => "nod",
            Self::OpenGripper => "open_gripper",
            Self::CloseGripper => "close_gripper",
            Self::Demo => "demo",
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::TurnLeft => "turn_left",
            Self::TurnRight => "turn_right",
            Self::Stop => "stop",
        }
    }

    pub fn from_id(id: &str, robot: RobotKind) -> Result<Self> {
        let id = id.trim().to_ascii_lowercase();
        let prim = match id.as_str() {
            "home" | "reset" => Self::Home,
            "wave" => Self::Wave,
            "reach" => Self::Reach,
            "nod" => Self::Nod,
            "open_gripper" => Self::OpenGripper,
            "close_gripper" => Self::CloseGripper,
            "demo" => Self::Demo,
            "forward" => Self::Forward,
            "backward" => Self::Backward,
            "turn_left" => Self::TurnLeft,
            "turn_right" => Self::TurnRight,
            "stop" => Self::Stop,
            other => bail!("unknown primitive id '{other}'"),
        };
        if !prim.allowed_on(robot) {
            bail!("primitive '{}' not valid for robot {:?}", prim.id(), robot);
        }
        Ok(prim)
    }

    pub fn allowed_on(self, robot: RobotKind) -> bool {
        match robot {
            RobotKind::Arm => matches!(
                self,
                Self::Home
                    | Self::Wave
                    | Self::Reach
                    | Self::Nod
                    | Self::OpenGripper
                    | Self::CloseGripper
                    | Self::Demo
            ),
            RobotKind::DiffCar => matches!(
                self,
                Self::Home
                    | Self::Forward
                    | Self::Backward
                    | Self::TurnLeft
                    | Self::TurnRight
                    | Self::Stop
                    | Self::Demo
            ),
        }
    }

    pub fn parse(instruction: &str) -> Result<Self> {
        Self::parse_for(instruction, RobotKind::Arm)
    }

    pub fn parse_for(instruction: &str, robot: RobotKind) -> Result<Self> {
        let steps = Self::parse_sequence_for(instruction, robot)?;
        steps
            .into_iter()
            .next()
            .map(|(p, _)| p)
            .ok_or_else(|| anyhow::anyhow!("empty parse"))
    }

    /// Parse one or more primitives (split on 再/然后/接着/then/and/,/，).
    pub fn parse_sequence_for(
        instruction: &str,
        robot: RobotKind,
    ) -> Result<Vec<(MotionPrimitive, f64)>> {
        let t = instruction.trim();
        if t.is_empty() {
            bail!("empty instruction");
        }
        let parts = split_instruction_parts(t);

        let mut steps = Vec::new();
        for part in parts {
            if let Ok(step) = Self::parse_one_segment(part, robot) {
                steps.push(step);
            }
        }
        if steps.is_empty() {
            steps = Self::scan_keywords_in_order(t, robot);
        }
        if steps.is_empty() {
            bail!(
                "unrecognized instruction for {:?}: '{instruction}'",
                robot
            );
        }
        Ok(steps)
    }

    /// Returns (primitive, scale). For turns, scale is multiples of 90° (180° → 2.0).
    fn parse_one_segment(segment: &str, robot: RobotKind) -> Result<(MotionPrimitive, f64)> {
        let t = segment.trim().to_lowercase();
        if t.is_empty() {
            bail!("empty");
        }
        let rules = rules_for(robot);
        let mut prim = None;
        for (keys, p) in rules {
            if keys.iter().any(|k| t.contains(k)) {
                prim = Some(*p);
                break;
            }
        }
        let prim = prim.ok_or_else(|| anyhow::anyhow!("unrecognized segment '{segment}'"))?;
        let scale = scale_from_segment(&t, prim);
        Ok((prim, scale))
    }

    fn scan_keywords_in_order(text: &str, robot: RobotKind) -> Vec<(MotionPrimitive, f64)> {
        let t = text.to_lowercase();
        let mut hits: Vec<(usize, MotionPrimitive)> = Vec::new();
        for (keys, prim) in rules_for(robot) {
            for k in keys.iter() {
                if let Some(idx) = t.find(*k) {
                    hits.push((idx, *prim));
                    break;
                }
            }
        }
        hits.sort_by_key(|(i, _)| *i);
        let mut out = Vec::new();
        let mut last: Option<MotionPrimitive> = None;
        for i in 0..hits.len() {
            let (idx, p) = hits[i];
            if last == Some(p) {
                continue;
            }
            let end = hits
                .iter()
                .skip(i + 1)
                .map(|(j, _)| *j)
                .next()
                .unwrap_or(t.len());
            let slice = &t[idx..end.min(t.len())];
            let scale = scale_from_segment(slice, p);
            out.push((p, scale));
            last = Some(p);
        }
        out
    }
}

/// Parse magnitude: "180度" / "90 degrees" / "半圈" / "一圈"; default 1.0 (= 90° for turns).
fn scale_from_segment(segment: &str, prim: MotionPrimitive) -> f64 {
    let t = segment.to_lowercase();
    if matches!(
        prim,
        MotionPrimitive::TurnLeft | MotionPrimitive::TurnRight
    ) {
        if t.contains("半圈") || t.contains("half") {
            return 2.0; // 180°
        }
        if t.contains("一圈") || t.contains("整圈") || t.contains("full circle") {
            return 4.0; // 360°
        }
        if let Some(deg) = parse_number_before_unit(&t, &["度", "degrees", "degree", "°"]) {
            return (deg / 90.0).clamp(0.2, 4.0);
        }
        if let Some(rad) = parse_number_before_unit(&t, &["rad", "弧度"]) {
            return (rad / std::f64::consts::FRAC_PI_2).clamp(0.2, 4.0);
        }
    }
    if matches!(
        prim,
        MotionPrimitive::Forward | MotionPrimitive::Backward
    ) {
        if let Some(m) = parse_number_before_unit(&t, &["米", "m", "公尺"]) {
            // default forward dist is 0.35m at scale 1
            return (m / 0.35).clamp(0.2, 6.0);
        }
        if t.contains("一点") || t.contains("一点点") || t.contains("slightly") {
            return 0.6;
        }
    }
    1.0
}

fn parse_number_before_unit(text: &str, units: &[&str]) -> Option<f64> {
    for unit in units {
        if let Some(pos) = text.find(unit) {
            let before = &text[..pos];
            // Take trailing digits / decimal from before.
            let bytes = before.as_bytes();
            let end = bytes.len();
            let mut start = end;
            while start > 0 {
                let c = bytes[start - 1] as char;
                if c.is_ascii_digit() || c == '.' {
                    start -= 1;
                } else {
                    break;
                }
            }
            if start < end {
                if let Ok(v) = before[start..end].parse::<f64>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn rules_for(robot: RobotKind) -> &'static [(&'static [&'static str], MotionPrimitive)] {
    match robot {
        RobotKind::Arm => &[
            (&["open gripper", "open the gripper", "张开夹爪", "打开夹爪", "张开手"], MotionPrimitive::OpenGripper),
            (&["close gripper", "close the gripper", "闭合夹爪", "关闭夹爪", "握住", "抓取"], MotionPrimitive::CloseGripper),
            (&["home", "reset", "零位", "回零", "回到初始", "复位"], MotionPrimitive::Home),
            (&["wave", "挥手", "打招呼", "招手"], MotionPrimitive::Wave),
            (&["reach", "伸出", "伸向"], MotionPrimitive::Reach),
            (&["nod", "点头"], MotionPrimitive::Nod),
            (&["demo", "演示", "动一下", "测试"], MotionPrimitive::Demo),
        ],
        RobotKind::DiffCar => &[
            (&["home", "reset", "回原点", "复位", "回零"], MotionPrimitive::Home),
            (&["backward", "后退", "倒车"], MotionPrimitive::Backward),
            (&["forward", "前进", "向前", "往前"], MotionPrimitive::Forward),
            (&["turn left", "左转", "向左转"], MotionPrimitive::TurnLeft),
            (&["turn right", "右转", "向右转"], MotionPrimitive::TurnRight),
            (&["stop", "停下", "停车", "停止"], MotionPrimitive::Stop),
            (&["demo", "演示", "转一圈", "动一下"], MotionPrimitive::Demo),
        ],
    }
}

fn split_instruction_parts(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = text;
    let connectors = ["然后", "接着", "and then", "then", "再", "，", ",", "；", ";"];
    while !rest.is_empty() {
        let mut best: Option<(usize, usize)> = None; // (index, sep_len)
        for sep in connectors {
            if let Some(i) = rest.find(sep) {
                let better = match best {
                    None => true,
                    Some((bi, _)) => i < bi,
                };
                if better {
                    best = Some((i, sep.len()));
                }
            }
        }
        if let Some((i, n)) = best {
            let head = rest[..i].trim();
            if !head.is_empty() {
                parts.push(head);
            }
            rest = rest[i + n..].trim();
        } else {
            let tail = rest.trim();
            if !tail.is_empty() {
                parts.push(tail);
            }
            break;
        }
    }
    if parts.is_empty() {
        parts.push(text.trim());
    }
    parts
}

pub struct InstructionPolicy {
    pub instruction: String,
    pub joint_names: Vec<String>,
    pub robot: RobotKind,
}

impl InstructionPolicy {
    pub fn new(instruction: impl Into<String>) -> Self {
        Self {
            instruction: instruction.into(),
            joint_names: (1..=6).map(|i| format!("joint_{i}")).collect(),
            robot: RobotKind::Arm,
        }
    }

    pub fn with_robot(mut self, robot: RobotKind) -> Self {
        self.robot = robot;
        self
    }

    pub fn with_joint_names(mut self, names: Vec<String>) -> Self {
        self.joint_names = names;
        self
    }
}

#[async_trait]
impl Policy for InstructionPolicy {
    fn name(&self) -> &str {
        "instruction"
    }

    async fn propose(&self, state: &WorldState) -> Result<ActionProposal> {
        let text = state
            .task_context
            .get("instruction")
            .and_then(|v| v.as_str())
            .unwrap_or(self.instruction.as_str());
        let steps = MotionPrimitive::parse_sequence_for(text, self.robot)?;
        let dof = state.joints().dof().max(1).min(16);
        let start: Vec<f64> = (0..dof)
            .map(|i| state.joints().positions.get(i).copied().unwrap_or(0.0))
            .collect();
        let grip = state.gripper_open();
        let waypoints = build_waypoint_sequence(self.robot, &steps, &start, grip);
        let prim_ids: Vec<&str> = steps.iter().map(|(p, _)| p.id()).collect();

        let resources = match self.robot {
            RobotKind::Arm => vec![ResourceKind::Arm, ResourceKind::Gripper],
            RobotKind::DiffCar => vec![ResourceKind::Base],
        };

        Ok(ActionProposal {
            id: format!("instr-{}-{}", prim_ids.join("+"), now_us()),
            stamp_us: now_us(),
            source: self.name().into(),
            waypoints,
            confidence: 0.9,
            required_resources: resources,
            metadata: serde_json::json!({
                "instruction": text,
                "primitives": prim_ids,
                "robot": self.robot.as_str(),
            }),
        })
    }
}

pub fn build_waypoints_for(
    robot: RobotKind,
    prim: MotionPrimitive,
    start: &[f64],
    grip: f64,
    scale: f64,
) -> Vec<JointWaypoint> {
    match robot {
        RobotKind::Arm => build_arm_waypoints(prim, start, grip, scale),
        RobotKind::DiffCar => build_car_waypoints(prim, start, scale),
    }
}

/// Chain multiple primitives; each step starts from the previous end pose.
pub fn build_waypoint_sequence(
    robot: RobotKind,
    steps: &[(MotionPrimitive, f64)],
    start: &[f64],
    grip: f64,
) -> Vec<JointWaypoint> {
    let mut all = Vec::new();
    let mut t_offset = 0.0;
    let mut cur = start.to_vec();
    for (i, (prim, scale)) in steps.iter().enumerate() {
        let mut wps = build_waypoints_for(robot, *prim, &cur, grip, *scale);
        if wps.is_empty() {
            continue;
        }
        if let Some(last) = wps.last() {
            cur = last.positions.clone();
        }
        for wp in &mut wps {
            wp.t_sec += t_offset;
        }
        // Drop duplicated start pose when stitching.
        if i > 0 && wps.len() > 1 {
            wps.remove(0);
        }
        if let Some(last_t) = wps.last().map(|w| w.t_sec) {
            t_offset = last_t + 0.1;
        }
        all.extend(wps);
    }
    all
}

fn build_arm_waypoints(
    prim: MotionPrimitive,
    start: &[f64],
    grip: f64,
    scale: f64,
) -> Vec<JointWaypoint> {
    let dof = start.len();
    let s = scale.clamp(0.2, 3.0);
    let mut home = vec![0.0; dof];
    if dof > 1 {
        home[1] = 0.15;
    }
    let clamp_all = |v: Vec<f64>| -> Vec<f64> {
        v.into_iter().map(|x| x.clamp(-2.2, 2.2)).collect()
    };

    match prim {
        MotionPrimitive::Home => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: start.to_vec(),
                gripper_open: Some(grip),
            },
            JointWaypoint {
                t_sec: 1.5,
                positions: clamp_all(home),
                gripper_open: Some(0.5),
            },
        ],
        MotionPrimitive::Wave => {
            let mut lift = start.to_vec();
            if dof > 1 {
                lift[1] = 0.7 * s.min(1.5);
            }
            if dof > 2 {
                lift[2] = -0.5 * s.min(1.5);
            }
            let mut left = lift.clone();
            let mut right = lift.clone();
            if dof > 0 {
                left[0] = 1.1 * s.min(1.5);
                right[0] = -1.1 * s.min(1.5);
            }
            if dof > 3 {
                left[3] = 0.8;
                right[3] = -0.8;
            }
            vec![
                JointWaypoint {
                    t_sec: 0.0,
                    positions: start.to_vec(),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 0.8,
                    positions: clamp_all(lift),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 1.6,
                    positions: clamp_all(left.clone()),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 2.4,
                    positions: clamp_all(right.clone()),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 3.2,
                    positions: clamp_all(left),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 4.0,
                    positions: clamp_all(right),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 4.8,
                    positions: start.to_vec(),
                    gripper_open: Some(1.0),
                },
            ]
        }
        MotionPrimitive::Reach => {
            let mut mid = start.to_vec();
            if dof > 1 {
                mid[1] = (start[1] + 0.4 * s).clamp(-2.0, 2.0);
            }
            if dof > 2 {
                mid[2] = (start[2] - 0.25 * s).clamp(-2.0, 2.0);
            }
            vec![
                JointWaypoint {
                    t_sec: 0.0,
                    positions: start.to_vec(),
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 1.2,
                    positions: mid,
                    gripper_open: Some(1.0),
                },
            ]
        }
        MotionPrimitive::Nod => {
            let mut down = start.to_vec();
            if dof > 4 {
                down[4] = (start[4] + 0.35 * s).clamp(-2.0, 2.0);
            } else if dof > 1 {
                down[1] = (start[1] + 0.25 * s).clamp(-2.0, 2.0);
            }
            vec![
                JointWaypoint {
                    t_sec: 0.0,
                    positions: start.to_vec(),
                    gripper_open: Some(grip),
                },
                JointWaypoint {
                    t_sec: 0.5,
                    positions: down,
                    gripper_open: Some(grip),
                },
                JointWaypoint {
                    t_sec: 1.0,
                    positions: start.to_vec(),
                    gripper_open: Some(grip),
                },
            ]
        }
        MotionPrimitive::OpenGripper => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: start.to_vec(),
                gripper_open: Some(grip),
            },
            JointWaypoint {
                t_sec: 0.8,
                positions: start.to_vec(),
                gripper_open: Some(1.0),
            },
        ],
        MotionPrimitive::CloseGripper => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: start.to_vec(),
                gripper_open: Some(grip),
            },
            JointWaypoint {
                t_sec: 0.8,
                positions: start.to_vec(),
                gripper_open: Some(0.0),
            },
        ],
        MotionPrimitive::Demo => {
            let mut mid = start.to_vec();
            for (i, p) in mid.iter_mut().enumerate() {
                let delta = if i % 2 == 0 { 0.3 } else { -0.2 };
                *p = (*p + delta * s).clamp(-2.0, 2.0);
            }
            vec![
                JointWaypoint {
                    t_sec: 0.0,
                    positions: start.to_vec(),
                    gripper_open: Some(grip),
                },
                JointWaypoint {
                    t_sec: 1.0,
                    positions: mid,
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 2.0,
                    positions: start.to_vec(),
                    gripper_open: Some(0.0),
                },
            ]
        }
        _ => vec![JointWaypoint {
            t_sec: 0.0,
            positions: start.to_vec(),
            gripper_open: Some(grip),
        }],
    }
}

/// Planar base joints expected as [root_x, root_y, root_yaw].
fn build_car_waypoints(prim: MotionPrimitive, start: &[f64], scale: f64) -> Vec<JointWaypoint> {
    let s = scale.clamp(0.2, 3.0);
    let x0 = start.first().copied().unwrap_or(0.0);
    let y0 = start.get(1).copied().unwrap_or(0.0);
    let yaw0 = start.get(2).copied().unwrap_or(0.0);
    let pad = |x, y, yaw| {
        let mut v = vec![x, y, yaw];
        while v.len() < start.len() {
            v.push(0.0);
        }
        v.truncate(start.len().max(3));
        v
    };
    let dist = 0.35 * s;
    // scale 1.0 = 90°, so "左转180度" → scale 2 → π rad
    let turn = std::f64::consts::FRAC_PI_2 * s;

    match prim {
        MotionPrimitive::Home => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: pad(x0, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 2.0,
                positions: pad(0.0, 0.0, 0.0),
                gripper_open: None,
            },
        ],
        MotionPrimitive::Forward => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: pad(x0, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 1.5,
                positions: pad(x0 + dist * yaw0.cos(), y0 + dist * yaw0.sin(), yaw0),
                gripper_open: None,
            },
        ],
        MotionPrimitive::Backward => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: pad(x0, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 1.5,
                positions: pad(x0 - dist * yaw0.cos(), y0 - dist * yaw0.sin(), yaw0),
                gripper_open: None,
            },
        ],
        MotionPrimitive::TurnLeft => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: pad(x0, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 1.2,
                positions: pad(x0, y0, yaw0 + turn),
                gripper_open: None,
            },
        ],
        MotionPrimitive::TurnRight => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: pad(x0, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 1.2,
                positions: pad(x0, y0, yaw0 - turn),
                gripper_open: None,
            },
        ],
        MotionPrimitive::Stop => vec![JointWaypoint {
            t_sec: 0.0,
            positions: pad(x0, y0, yaw0),
            gripper_open: None,
        }],
        MotionPrimitive::Demo => vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: pad(x0, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 1.0,
                positions: pad(x0 + dist, y0, yaw0),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 2.0,
                positions: pad(x0 + dist, y0, yaw0 + turn),
                gripper_open: None,
            },
            JointWaypoint {
                t_sec: 3.0,
                positions: pad(x0 + 2.0 * dist, y0 + 0.2 * s, yaw0 + turn),
                gripper_open: None,
            },
        ],
        _ => vec![JointWaypoint {
            t_sec: 0.0,
            positions: pad(x0, y0, yaw0),
            gripper_open: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::{JointState, ProprioState};

    #[test]
    fn parses_chinese_wave() {
        assert_eq!(MotionPrimitive::parse("请挥手").unwrap(), MotionPrimitive::Wave);
    }

    #[test]
    fn parses_car_sequence_forward_then_left() {
        let steps =
            MotionPrimitive::parse_sequence_for("向前走一点再左转", RobotKind::DiffCar).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].0, MotionPrimitive::Forward);
        assert_eq!(steps[1].0, MotionPrimitive::TurnLeft);
    }

    #[test]
    fn parses_turn_180_degrees() {
        let steps = MotionPrimitive::parse_sequence_for("向前走一点再左转180度", RobotKind::DiffCar)
            .unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].0, MotionPrimitive::TurnLeft);
        assert!((steps[1].1 - 2.0).abs() < 1e-6, "scale={}", steps[1].1);
        let wps = build_waypoint_sequence(RobotKind::DiffCar, &steps, &[0.0, 0.0, 0.0], 0.0);
        let yaw = wps.last().unwrap().positions[2];
        assert!(
            (yaw - std::f64::consts::PI).abs() < 0.05,
            "expected ~π yaw, got {yaw}"
        );
    }

    #[test]
    fn parses_home() {
        assert_eq!(MotionPrimitive::parse("home").unwrap(), MotionPrimitive::Home);
    }

    #[tokio::test]
    async fn proposes_wave_waypoints() {
        let policy = InstructionPolicy::new("wave");
        let state = WorldState {
            stamp_us: 0,
            proprio: ProprioState::new(
                JointState::new((1..=6).map(|i| format!("joint_{i}")).collect(), vec![0.0; 6]),
                0.5,
            ),
            annotations: vec![],
            modality_keys: vec![],
            primary_image_uri: None,
            task_context: serde_json::json!({ "instruction": "挥手" }),
        };
        let p = policy.propose(&state).await.unwrap();
        assert!(p.waypoints.len() >= 3);
        assert_eq!(p.metadata["primitives"][0], "wave");
    }
}
