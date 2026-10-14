//! Plain unit-style tests of the scenario helpers; no fake terminal required.
//! Calls to Given/When/Then here exercise the API under test, not a convention
//! for writing unit tests.

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use support::scenario::{and, given, scenario, then, when};

#[test]
fn closures_capture_borrow_move_and_return_values_once() {
    let mut executions = Vec::new();
    let owned = String::from("fixture");
    scenario("closures behave like ordinary Rust", || {
        let mut value = given("an owned fixture", || {
            executions.push("Given");
            owned
        });
        let suffix = and("another starting condition", || {
            executions.push("And Given");
            String::from(" updated")
        });
        when("the fixture changes", || {
            executions.push("When");
            value.push_str(&suffix);
        });
        and("the action finishes", || {
            executions.push("And When");
            value.push('!');
        });
        then("the result matches", || {
            executions.push("Then");
            assert_eq!(value, "fixture updated!");
        });
        and("the fixture is still available", || {
            executions.push("And Then");
            assert!(!value.is_empty());
        });
    });
    assert_eq!(
        executions,
        ["Given", "And Given", "When", "And When", "Then", "And Then"]
    );
}

fn complete_scenario() {
    scenario("a fresh scenario", || {
        given("starting conditions", || {});
        when("an action happens", || {});
        then("its result is checked", || {});
    });
}

fn rejects(body: impl FnOnce()) -> String {
    let error = catch_unwind(AssertUnwindSafe(body)).expect_err("invalid story must fail");
    let message = if let Some(message) = error.downcast_ref::<String>() {
        message.clone()
    } else {
        error.downcast_ref::<&str>().unwrap().to_string()
    };
    complete_scenario(); // Unwinding must not leave context behind.
    message
}

#[test]
fn assertions_keep_the_original_panic_payload_and_release_context() {
    #[derive(Debug, PartialEq)]
    struct Failure(u8);

    let error = catch_unwind(|| {
        scenario("a failing assertion", || {
            given("a fixture", || {});
            when("an action happens", || {});
            then("an assertion fails", || panic_any(Failure(42)));
        });
    })
    .unwrap_err();
    assert_eq!(*error.downcast::<Failure>().unwrap(), Failure(42));
    complete_scenario();
}

#[test]
fn assertion_failures_keep_the_original_message() {
    let message = rejects(|| {
        scenario("A failing assertion retains its details", || {
            let value = given("a value of one", || 1);
            when("the value is left unchanged", || {});
            then("the value should be two", || {
                assert_eq!(value, 2, "expected total");
            });
        });
    });
    assert!(message.contains("expected total"), "{message}");
    assert!(message.contains("left: 1"), "{message}");
    assert!(message.contains("right: 2"), "{message}");
}

#[test]
fn failures_in_setup_and_actions_release_context() {
    for phase in ["Given", "When", "And"] {
        let message = rejects(|| {
            scenario("a failing step", || {
                given("setup", || {
                    assert_ne!(phase, "Given", "setup failed");
                });
                when("action", || {
                    assert_ne!(phase, "When", "action failed");
                });
                and("more action", || panic!("And failed"));
            });
        });
        assert!(message.contains("failed"), "{message}");
    }
}

#[test]
fn steps_require_a_scenario_and_correct_phase_order() {
    assert!(rejects(|| given("outside a scenario", || {})).contains("inside a scenario"));
    for phase in ["When", "Then", "And"] {
        let message = rejects(|| {
            scenario("missing setup", || match phase {
                "When" => when("too early", || {}),
                "Then" => then("too early", || {}),
                _ => and("too early", || {}),
            });
        });
        assert!(message.contains("not allowed after Start"), "{message}");
    }
    assert!(
        rejects(|| {
            scenario("missing action", || {
                given("setup", || {});
                then("too early", || {});
            });
        })
        .contains("not allowed after Given")
    );
}

#[test]
fn new_behaviours_cannot_be_appended_after_assertions() {
    let message = rejects(|| {
        scenario("one behaviour only", || {
            given("setup", || {});
            when("an action", || {});
            then("a result", || {});
            when("another behaviour", || {});
        });
    });
    assert!(message.contains("not allowed after Then"), "{message}");
}

#[test]
fn scenarios_must_include_the_then_phase() {
    assert!(rejects(|| scenario("empty", || {})).contains("must include Given, When and Then"));
    for with_action in [false, true] {
        let message = rejects(|| {
            scenario("an unfinished story", || {
                given("setup", || {});
                if with_action {
                    when("an action", || {});
                }
            });
        });
        assert!(message.contains("must include Given, When and Then"));
    }
}

#[test]
fn scenarios_and_steps_cannot_be_nested() {
    assert!(
        rejects(|| scenario("outer", || scenario("inner", || {})))
            .contains("scenarios must not be nested")
    );
    assert!(
        rejects(|| scenario("nested steps", || given("outer", || given("inner", || {}))))
            .contains("steps must not be nested")
    );
}

#[test]
fn concurrent_scenarios_do_not_share_context() {
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let barrier = &barrier;
            scope.spawn(move || {
                scenario("a concurrent story", || {
                    given("both stories are active", || {
                        barrier.wait();
                    });
                    when("an action happens", || {});
                    then("its result is checked", || {});
                });
            });
        }
    });
}
