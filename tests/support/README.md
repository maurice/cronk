# Writing readable UI scenarios

Functional tests should tell one story: the starting conditions, one behaviour,
and its expected consequences. The helpers here execute ordinary Rust closures;
this is Given/When/Then-style Rust, not a parser for `.feature` files or a new
test runner. Each scenario stays inside its own `#[test]`, so nextest can select
and run it independently.

## Where this style belongs

**Scenario syntax is for functional behaviour tests, not a project-wide rule.**
Choose by what the test proves, not where its file lives:

| Kind of test | Style |
| --- | --- |
| Functional UI behaviour: starring an issue, changing project visibility, scrolling logs | Named scenario steps when they help explain the user's story. |
| Unit tests: parsing, sorting, formatting, configuration calculations, individual helpers | Ordinary `#[test]` functions and direct assertions. No scenario wrapper required. |
| Property-based, performance, protocol and component tests | Keep the style appropriate to the invariant, measurement or contract. |

Rust's `tests/` directory contains separate test binaries; it is not a promise
that every test there is functional or end-to-end. For example,
`pipeline_refresh_cadence_defaults_to_twice_the_list_cadence` stays a plain
configuration test even though it shares a file with pipeline UI scenarios.
The tests in `tests/scenarios.rs` are plain unit-style tests of the step helpers
themselves; their calls to the helpers are the code under test.

The current UI scenarios run the integrated app in a fake terminal with demo
data. They are headless functional tests, not full end-to-end tests against
GitLab. Shared UI mechanics can be reused **without** importing scenario steps.
There is no goal of converting the entire test suite to Given/When/Then.

## A complete example

```rust
mod support;

use support::{
    scenario::{and, given, scenario, then, when},
    ui::{demo_config, key, mount_demo, pump_render, render_text, viewport},
};
use tui_lipan::prelude::KeyCode;

#[test]
fn starring_an_issue_reveals_the_starred_tab() {
    scenario("Starring an issue reveals the Starred tab", || {
        let mut ui = given("an issue list without starred items", || {
            let mut config = demo_config();
            config.active_tab = 2;
            let mut ui = mount_demo(config, None, viewport(110, 40));
            pump_render(&mut ui, 3);
            ui
        });

        when("I press s on the selected issue", || {
            key(&mut ui, KeyCode::Char('s'));
            pump_render(&mut ui, 3);
        });

        then("exactly one item is starred", || {
            assert_eq!(ui.state().config.starred.len(), 1);
        });
        and("the Starred tab is visible", || {
            assert!(render_text(&mut ui).contains("★ Starred"));
        });
    });
}
```

`|| { ... }` is a closure with no parameters. It can use surrounding variables:
the action borrows `ui`, for example. `given` returns the value its closure
creates; returning a tuple is useful when setup also produces an item ID or a
temporary directory. Keep temporary directories alive for the whole scenario.

## Keep the story focused

- Use one `scenario` per `#[test]`, with a fresh fixture in `given`. The language
  tracks step context; it does not reset application state for you.
- Put the starting conditions in `given`, the behaviour under test in `when`,
  and ordinary assertions in `then` and `and`. Setup assertions are welcome
  when they verify an important precondition.
- Several assertions can describe the consequences of one action. Do not add
  another unrelated action after checking the first result: write another test.
- `and` extends the current phase. It can add setup, another part of the same
  action, or another expected consequence. It does not start a new phase.
- Step order is checked at runtime: Given → When → Then, with optional And
  steps. Missing phases, nested scenarios/steps, and a new When after Then fail.
  Assertions are not restricted to read-only code by the type system.
- Keep steps on the scenario's thread. If a step starts workers, join them and
  propagate their failures inside that step.
- Name steps in terms of behaviour, not implementation messages. Keep
  scenario-specific navigation and fixture preparation in the relevant suite;
  shared support is for genuinely common mechanics.

Failure output includes the executed steps, the failed step and the scenario
name. The original assertion panic, source location and backtrace are preserved:
the helpers do not catch or replace the panic or install a global panic hook.
Nextest captures this output and shows it for failed tests.

## Drawing and processing updates are different

`mount_demo` creates a manually focused demo app with an explicit viewport. A
supplied workspace path is saved if absent; an existing config is left alone.
It does **not** pump updates or render. Input helpers also do not settle,
advance time, or sleep implicitly.

| Helper | What it does |
| --- | --- |
| `pump_render(ui, rounds)` | Process queued updates, then draw, for the specified number of rounds. The last draw can queue viewport callbacks. |
| `render_pump(ui, rounds)` | Draw, then process updates, including callbacks from that draw. The last updates may need another draw. |
| `frame_text(ui)` | Read the last drawn frame without doing more work. |
| `render_text(ui)` | Draw a fresh frame and read it, without draining callbacks. |
| `key` / `modified_key` / `mouse` | Send an input event, without additional settling. |
| `element_rect(ui, key)` | Find a mounted element, with its key in the failure message if missing. |

Small local wrappers such as `settle` and `key` can preserve a suite's existing
timing convention. Pipelines use three draw/process rounds, starred items use
three process/draw rounds, and job logs use four process/draw rounds. Do not
flatten these differences during extraction or add extra rounds just to make a
failure disappear. Advance the fake clock explicitly when time is part of the
scenario.

## Run a scenario

```sh
just test --test starred -E 'test(starring_an_issue_reveals_the_starred_tab)'
just test --test pipelines --test starred --test job_logs --test scenarios
```

This first slice shares mechanics across those three UI suites and converts
eight functional scenarios. Wider helper extraction is follow-up work; adopting
the scenario style is a separate, opt-in choice for suitable functional tests.
