//! Selected command dialects compiled before dispatch. Lua queueing, routing,
//! and a native call returning are separate facts; none proves a game effect.
use crate::{platform::Ecosystem, vm_protocol::Action};
use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};

pub const MAX_INPUT_BYTES: usize = 1023;
const MAX_BATCHES: usize = 256;
const MAX_SCRIPT_BYTES: usize = 65536;
const MAX_STEPS: usize = 128;
const PUMP_BUDGET: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub origin: Ecosystem,
    pub mode: i32,
    pub bytes: Vec<u8>,
}
impl Input {
    pub fn validate(&self) -> Result<(), String> {
        if !(0..=7).contains(&self.mode) {
            return Err("Game input mode must be between 0 and 7".into());
        }
        if self.bytes.is_empty() || self.bytes.len() > MAX_INPUT_BYTES || self.bytes.contains(&0) {
            return Err("Game input must contain 1..1023 bytes without NUL".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Input(Input),
    Chat { mode: u8, bytes: Vec<u8> },
    Wait(Duration),
}
pub trait Backend {
    fn input(&mut self, input: &Input) -> Result<(), String>;
    fn chat(&mut self, _mode: u8, _bytes: &[u8]) -> Result<(), String> {
        Err("Game chat backend is unavailable".into())
    }
}
fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &bytes[start..end]
}
fn input(origin: Ecosystem, mode: i32, bytes: &[u8]) -> Result<Step, String> {
    let input = Input {
        origin,
        mode,
        bytes: bytes.to_vec(),
    };
    input.validate()?;
    Ok(Step::Input(input))
}
fn console(origin: Ecosystem, bytes: &[u8]) -> Result<Vec<Step>, String> {
    if bytes.len() > MAX_SCRIPT_BYTES || bytes.contains(&0) {
        return Err("Command script exceeds limits or contains NUL".into());
    }
    let mut parts = Vec::new();
    let (mut start, mut index, mut quote) = (0, 0, None);
    while index < bytes.len() {
        let byte = bytes[index];
        // CP932 double-byte characters may contain an ASCII backslash as the
        // trailing byte. Preserve the pair instead of treating it as syntax.
        if matches!(byte, 0x81..=0x9f | 0xe0..=0xfc) && index + 1 < bytes.len() {
            index += 2;
            continue;
        }
        if byte == b'\\'
            && index + 1 < bytes.len()
            && matches!(bytes[index + 1], b'\'' | b'"' | b';' | b'\\')
        {
            index += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(byte);
            }
        } else if byte == b';' && quote.is_none() {
            parts.push(&bytes[start..index]);
            start = index + 1;
        }
        index += 1;
    }
    if quote.is_some() {
        return Err("Unterminated command quote".into());
    }
    parts.push(&bytes[start..]);
    if parts.len() > MAX_STEPS {
        return Err("Command script exceeds step limit".into());
    }
    let mut steps = Vec::new();
    for part in parts {
        let part = trim(part);
        if part.is_empty() {
            continue;
        }
        let end = part
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(part.len());
        let verb = &part[..end];
        let mut argument = trim(&part[end..]);
        if argument.len() >= 2
            && matches!(argument[0], b'\'' | b'"')
            && argument.last() == Some(&argument[0])
        {
            argument = &argument[1..argument.len() - 1];
        }
        if verb.eq_ignore_ascii_case(b"input") || verb.eq_ignore_ascii_case(b"keyboard_sendstring")
        {
            steps.push(input(origin, 1, argument)?);
        } else if verb.eq_ignore_ascii_case(b"wait") {
            let seconds: f64 = std::str::from_utf8(argument)
                .map_err(|_| "Invalid wait duration")?
                .parse()
                .map_err(|_| "Invalid wait duration")?;
            if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
                return Err("Wait duration must be 0..86400 seconds".into());
            }
            steps.push(Step::Wait(Duration::from_secs_f64(seconds)));
        } else {
            return Err(format!(
                "Unsupported console command: {}",
                String::from_utf8_lossy(verb)
            ));
        }
    }
    Ok(steps)
}
pub fn compile(action: Action) -> Result<Vec<Step>, String> {
    match action {
        Action::WindowerCommand { bytes } => console(Ecosystem::Windower4, &bytes),
        Action::NativeCommand { bytes } => console(Ecosystem::Native, &bytes),
        Action::AshitaCommand { mode, bytes } => {
            // Negative modes enter Ashita's parser, not the game's function.
            if mode < 0 {
                return Err("Ashita parser/script command modes are not implemented".into());
            }
            Ok(vec![input(Ecosystem::Ashita4, mode, &bytes)?])
        }
        Action::GameInput {
            origin,
            mode,
            bytes,
        } => Ok(vec![input(origin, mode, &bytes)?]),
        Action::Chat { mode, bytes } => {
            let mode = u8::try_from(mode).map_err(|_| "Chat mode must fit one byte")?;
            if bytes.len() > 4095 || bytes.contains(&0) {
                return Err("Chat text exceeds limits or contains NUL".into());
            }
            Ok(vec![Step::Chat { mode, bytes }])
        }
    }
}
struct Pending {
    owner: u64,
    steps: VecDeque<Step>,
    due: Duration,
    waiting: bool,
}
#[derive(Default)]
pub struct Dispatcher {
    pending: VecDeque<Pending>,
    last_time: Duration,
}
#[derive(Default, Debug)]
pub struct PumpReport {
    pub delivered: usize,
    pub failures: Vec<(u64, String)>,
}
impl Dispatcher {
    pub fn enqueue(&mut self, owner: u64, action: Action, now: Duration) -> Result<(), String> {
        if now < self.last_time {
            return Err("Command clock moved backwards".into());
        }
        let steps = compile(action)?;
        if steps.is_empty() {
            return Ok(());
        }
        if self.pending.len() >= MAX_BATCHES {
            return Err("Command queue is full".into());
        }
        self.pending.push_back(Pending {
            owner,
            steps: steps.into(),
            due: now,
            waiting: false,
        });
        Ok(())
    }
    pub fn cancel(&mut self, owner: u64) {
        self.pending.retain(|item| item.owner != owner);
    }
    pub fn pending_batches(&self) -> usize {
        self.pending.len()
    }
    pub fn pump(
        &mut self,
        now: Duration,
        backend: &mut impl Backend,
    ) -> Result<PumpReport, String> {
        if now < self.last_time {
            return Err("Command clock moved backwards".into());
        }
        self.last_time = now;
        let mut blocked = BTreeSet::new();
        let (mut budget, mut report) = (PUMP_BUDGET, PumpReport::default());
        for _ in 0..self.pending.len() {
            let Some(mut batch) = self.pending.pop_front() else {
                break;
            };
            if blocked.contains(&batch.owner) || batch.due > now || budget == 0 {
                blocked.insert(batch.owner);
                self.pending.push_back(batch);
                continue;
            }
            batch.waiting = false;
            while budget > 0 {
                let Some(step) = batch.steps.pop_front() else {
                    break;
                };
                budget -= 1;
                let result = match step {
                    Step::Wait(delay) => {
                        batch.due = now.saturating_add(delay);
                        batch.waiting = true;
                        break;
                    }
                    Step::Input(input) => backend.input(&input),
                    Step::Chat { mode, bytes } => backend.chat(mode, &bytes),
                };
                match result {
                    Ok(()) => report.delivered += 1,
                    Err(error) => {
                        report.failures.push((batch.owner, error));
                        // A native call may have had an effect before failing.
                        // Never retry it or the remainder implicitly.
                        batch.steps.clear();
                        break;
                    }
                }
            }
            if !batch.steps.is_empty() || batch.waiting {
                blocked.insert(batch.owner);
                self.pending.push_back(batch);
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Capture(Vec<Input>);
    impl Backend for Capture {
        fn input(&mut self, input: &Input) -> Result<(), String> {
            self.0.push(input.clone());
            Ok(())
        }
    }
    fn script(bytes: &[u8]) -> Action {
        Action::WindowerCommand {
            bytes: bytes.to_vec(),
        }
    }
    #[test]
    fn dialects_preserve_payload_mode_and_reject_parser_fallthrough() {
        assert_eq!(
            compile(script(b"input /ma \"Cure IV\" <me>;input '/echo a;b'")).unwrap(),
            vec![
                input(Ecosystem::Windower4, 1, b"/ma \"Cure IV\" <me>").unwrap(),
                input(Ecosystem::Windower4, 1, b"/echo a;b").unwrap()
            ]
        );
        let bytes = vec![b'/', 0x83, 0x5c, b';', 0xff];
        assert_eq!(
            compile(Action::AshitaCommand {
                mode: 2,
                bytes: bytes.clone()
            })
            .unwrap(),
            vec![input(Ecosystem::Ashita4, 2, &bytes).unwrap()]
        );
        for mode in [-3, -2, -1, 8] {
            assert!(
                compile(Action::AshitaCommand {
                    mode,
                    bytes: b"/echo x".to_vec()
                })
                .is_err()
            );
        }
        for bytes in [
            b"input ok;unknown x".as_slice(),
            b"input 'bad",
            b"wait NaN",
            b"wait -1",
            b"input a\0b",
        ] {
            assert!(compile(script(bytes)).is_err());
        }
    }
    #[test]
    fn waits_preserve_owner_order_without_blocking_other_addons_and_can_cancel() {
        let (mut dispatcher, mut backend) = (Dispatcher::default(), Capture::default());
        dispatcher
            .enqueue(
                1,
                script(b"input first;wait 1;input second"),
                Duration::ZERO,
            )
            .unwrap();
        dispatcher
            .enqueue(1, script(b"input third"), Duration::ZERO)
            .unwrap();
        dispatcher
            .enqueue(2, script(b"input independent"), Duration::ZERO)
            .unwrap();
        assert_eq!(
            dispatcher
                .pump(Duration::ZERO, &mut backend)
                .unwrap()
                .delivered,
            2
        );
        assert_eq!(
            dispatcher
                .pump(Duration::from_millis(999), &mut backend)
                .unwrap()
                .delivered,
            0
        );
        assert_eq!(
            dispatcher
                .pump(Duration::from_secs(1), &mut backend)
                .unwrap()
                .delivered,
            2
        );
        assert_eq!(
            backend
                .0
                .iter()
                .map(|v| v.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![b"first".as_slice(), b"independent", b"second", b"third"]
        );
        assert!(dispatcher.pump(Duration::ZERO, &mut backend).is_err());
        dispatcher
            .enqueue(1, script(b"wait 1;input cancelled"), Duration::from_secs(1))
            .unwrap();
        dispatcher.cancel(1);
        assert_eq!(dispatcher.pending_batches(), 0);
    }
    #[test]
    fn zero_wait_yields_and_backend_failures_do_not_retry() {
        let (mut dispatcher, mut backend) = (Dispatcher::default(), Capture::default());
        dispatcher
            .enqueue(1, script(b"wait 0;input next tick"), Duration::ZERO)
            .unwrap();
        assert_eq!(
            dispatcher
                .pump(Duration::ZERO, &mut backend)
                .unwrap()
                .delivered,
            0
        );
        assert_eq!(
            dispatcher
                .pump(Duration::ZERO, &mut backend)
                .unwrap()
                .delivered,
            1
        );
        struct Failed;
        impl Backend for Failed {
            fn input(&mut self, _: &Input) -> Result<(), String> {
                Err("unavailable".into())
            }
        }
        dispatcher
            .enqueue(1, script(b"input first;input second"), Duration::ZERO)
            .unwrap();
        let report = dispatcher.pump(Duration::ZERO, &mut Failed).unwrap();
        assert_eq!(report.delivered, 0);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(dispatcher.pending_batches(), 0);
    }
    #[test]
    fn trailing_wait_queue_limits_and_cp932_trailing_backslashes_are_preserved() {
        let (mut dispatcher, mut backend) = (Dispatcher::default(), Capture::default());
        dispatcher
            .enqueue(1, script(b"wait 1"), Duration::ZERO)
            .unwrap();
        dispatcher
            .enqueue(1, script(b"input after"), Duration::ZERO)
            .unwrap();
        assert_eq!(
            dispatcher
                .pump(Duration::ZERO, &mut backend)
                .unwrap()
                .delivered,
            0
        );
        assert_eq!(
            dispatcher
                .pump(Duration::from_secs(1), &mut backend)
                .unwrap()
                .delivered,
            1
        );
        let bytes = [b"input \"".as_slice(), &[0x83, 0x5c], b"\";input second"].concat();
        assert_eq!(
            compile(script(&bytes)).unwrap(),
            vec![
                input(Ecosystem::Windower4, 1, &[0x83, 0x5c]).unwrap(),
                input(Ecosystem::Windower4, 1, b"second").unwrap()
            ]
        );
        for owner in 0..MAX_BATCHES {
            dispatcher
                .enqueue(owner as u64, script(b"input x"), Duration::from_secs(1))
                .unwrap();
        }
        assert!(
            dispatcher
                .enqueue(9999, script(b"input overflow"), Duration::from_secs(1))
                .is_err()
        );
        assert_eq!(
            dispatcher
                .pump(Duration::from_secs(1), &mut backend)
                .unwrap()
                .delivered,
            PUMP_BUDGET
        );
        assert_eq!(dispatcher.pending_batches(), MAX_BATCHES - PUMP_BUDGET);
    }
}
