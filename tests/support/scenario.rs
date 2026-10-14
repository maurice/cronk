//! Executable Given/When/Then steps around ordinary Rust assertions.
//!
//! Context is local to the test thread. Closures can capture and return values;
//! assertions keep their original panic payload, location and backtrace. Guards
//! add the failed step/scenario to captured stderr while unwinding, without
//! catching panics or changing the process-wide panic hook.

use std::cell::RefCell;

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

struct Context {
    name: String,
    phase: Phase,
    step_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    Given,
    When,
    Then,
}

#[derive(Clone, Copy)]
enum Step {
    Given,
    When,
    Then,
    And,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Self::Given => "Given",
            Self::When => "When",
            Self::Then => "Then",
            Self::And => "And",
        }
    }
}

struct ScenarioGuard;

impl Drop for ScenarioGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            if let Some(context) = current.borrow_mut().take()
                && std::thread::panicking()
            {
                eprintln!("Failed scenario: {}", context.name);
            }
        });
    }
}

struct StepGuard(String);

impl Drop for StepGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            if let Some(context) = current.borrow_mut().as_mut() {
                context.step_active = false;
            }
        });
        if std::thread::panicking() {
            eprintln!("Failed step: {}", self.0);
        }
    }
}

/// Run one isolated story inside a normal `#[test]`.
#[track_caller]
pub fn scenario(description: &str, body: impl FnOnce()) {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none(), "scenarios must not be nested");
        *current = Some(Context {
            name: description.into(),
            phase: Phase::Start,
            step_active: false,
        });
    });
    let _guard = ScenarioGuard;
    eprintln!("Scenario: {description}");
    body();
    CURRENT.with(|current| {
        assert_eq!(
            current.borrow().as_ref().unwrap().phase,
            Phase::Then,
            "a scenario must include Given, When and Then"
        );
    });
}

/// Establish the starting conditions, returning any fixture the story needs.
#[track_caller]
pub fn given<T>(description: &str, body: impl FnOnce() -> T) -> T {
    step(Step::Given, description, body)
}

/// Perform the action whose consequences this scenario checks.
#[track_caller]
pub fn when(description: &str, body: impl FnOnce()) {
    step(Step::When, description, body);
}

/// Check the result with ordinary Rust assertions.
#[track_caller]
pub fn then(description: &str, body: impl FnOnce()) {
    step(Step::Then, description, body);
}

/// Extend the current Given, When or Then phase, not start another behaviour.
#[track_caller]
pub fn and<T>(description: &str, body: impl FnOnce() -> T) -> T {
    step(Step::And, description, body)
}

#[track_caller]
fn step<T>(kind: Step, description: &str, body: impl FnOnce() -> T) -> T {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let context = current
            .as_mut()
            .expect("Given/When/Then/And must run inside a scenario");
        assert!(!context.step_active, "steps must not be nested");
        context.phase = match (kind, context.phase) {
            (Step::Given, Phase::Start) => Phase::Given,
            (Step::When, Phase::Given) => Phase::When,
            (Step::Then, Phase::When) => Phase::Then,
            (Step::And, Phase::Given | Phase::When | Phase::Then) => context.phase,
            _ => panic!(
                "{} is not allowed after {:?}; use Given, When, Then, with And to extend a phase",
                kind.name(),
                context.phase
            ),
        };
        context.step_active = true;
    });
    let label = format!("{} {description}", kind.name());
    eprintln!("  {label}");
    let _guard = StepGuard(label);
    body()
}
