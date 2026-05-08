# Tasks: Live Connection Hot-Reload and check_config Tool

**Input**: Design documents from `/specs/034-live-connection-reload/`
**Repo**: `~/ws/iris-dev` (Rust — `crates/iris-dev-core` + `crates/iris-dev-bin`)
**Constitution**: Principle IV — unit tests first; Principle VII — zero new crates

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add new types and stubs, update `IrisTools` struct — no behavior change yet.

- [x] T001 Define `ConnectionSource` enum (`ConfigFile | EnvVars | IrisSelectContainer | AutoDiscovered`) in `crates/iris-dev-core/src/tools/mod.rs`
- [x] T002 Define `ConnectionState` struct (fields: `iris: Option<Arc<IrisConnection>>`, `source: ConnectionSource`, `config_file: Option<PathBuf>`, `loaded_at: SystemTime`, `write_tools_enabled: bool`, `config_parse_error: Option<String>`) in `crates/iris-dev-core/src/tools/mod.rs`
- [x] T003 Define `ConfigWatcher` struct (fields: `config_path: PathBuf`, `last_mtime: SystemTime`) in `crates/iris-dev-core/src/tools/mod.rs`
- [x] T004 Replace `IrisTools.iris: Option<Arc<IrisConnection>>` and `IrisTools.write_tools_enabled: bool` with `connection: Arc<Mutex<ConnectionState>>` and `config_watcher: Option<ConfigWatcher>` in `crates/iris-dev-core/src/tools/mod.rs` — update all constructor functions (`new`, `new_with_toolset`, `with_registry`, `with_registry_and_toolset`) to build `ConnectionState` from the `Option<IrisConnection>` param and wrap in `Arc::new(Mutex::new(...))`; also update the `with_registry_and_toolset()` signature to accept an additional `config_watcher: Option<ConfigWatcher>` parameter (T025 in `mcp.rs` passes this value; all other call sites pass `None`)
- [x] T005 Update `get_iris()` in `crates/iris-dev-core/src/tools/mod.rs` — change return type from `Result<&IrisConnection, McpError>` to `Result<Arc<IrisConnection>, McpError>`; lock `self.connection`, clone `iris` Arc out of `ConnectionState`, release lock
- [x] T006 Update all `self.write_tools_enabled` references in `crates/iris-dev-core/src/tools/mod.rs` to read from `self.connection.lock().unwrap().write_tools_enabled`
- [x] T007 [P] Create empty test stub `crates/iris-dev-core/tests/unit/test_live_reload.rs`
- [x] T008 [P] Create empty E2E test stub `crates/iris-dev-core/tests/integration/test_live_reload_e2e.rs`
- [x] T009 [P] Add `[[test]]` entries for both new test files to `crates/iris-dev-core/Cargo.toml`
- [x] T010 Verify `cargo check -p iris-dev-core` passes with all struct changes

**Checkpoint**: `cargo check` clean. New types defined. `get_iris()` returns `Arc<IrisConnection>`. All existing call sites compile.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Implement `check_reload()` — the lazy mtime check shared by all user stories. Tests first.

### Tests for Phase 2 (write first — must FAIL before implementation)

- [x] T011 [P] Write unit test: `ConnectionState::new_from_none()` returns correct defaults (`connected: false`, `write_tools_enabled: true`, `source: EnvVars`) in `tests/unit/test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T012 [P] Write unit test: `ConnectionSource` serializes to correct strings (`"config_file"`, `"env_vars"`, `"iris_select_container"`, `"auto_discovered"`) in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T013 [P] Write unit test: `ConfigWatcher::new(path)` initializes with current file mtime — call `stat()` at construction time in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T014 [P] Write unit test: `IrisTools` with `None` iris has `connection.lock().iris == None` and `write_tools_enabled == true` in `test_live_reload.rs` (WRITE FIRST, must FAIL)

### TDD Gate

- [x] T015 **GATE**: Confirm `cargo test --test test_live_reload` produces FAILURES (types exist but methods don't yet). Do not proceed until T011–T014 fail to compile or assert.

### Implementation for Phase 2

- [x] T016 Implement `ConnectionState::new_disconnected(source: ConnectionSource) -> Self` constructor in `crates/iris-dev-core/src/tools/mod.rs` — sets `iris: None`, `write_tools_enabled: true`, `loaded_at: SystemTime::now()`, `config_parse_error: None`
- [x] T017 Implement `ConnectionState::from_iris(iris: IrisConnection, source: ConnectionSource, config_file: Option<PathBuf>) -> Self` in `mod.rs` — sets `write_tools_enabled: iris.is_write_allowed()`, wraps iris in Arc, records timestamp
- [x] T018 Implement `IrisTools::check_reload(&self) -> ()` async method in `mod.rs`:
  - If `self.config_watcher` is None → return immediately
  - `stat()` the config path via `std::fs::metadata(&watcher.config_path)`
  - If mtime <= `watcher.last_mtime` → return immediately
  - Reload config via `load_workspace_config()`, build new `IrisConnection`, call `.probe().await`
  - On success: lock `self.connection`, replace with new `ConnectionState(source=ConfigFile)`, update `watcher.last_mtime`
  - On failure: lock `self.connection`, set `config_parse_error` only, preserve existing `iris`
- [x] T019 Verify T011–T014 all pass GREEN: `cargo test --test test_live_reload`

**Checkpoint**: All foundational unit tests green. `check_reload()` implemented.

---

## Phase 3: User Story 1 — Config file changes, session adapts silently (Priority: P1) 🎯 MVP

**Goal**: When `.iris-dev.toml` is modified, the very next tool call uses the new connection. Agent sees no error or interruption.

**Independent Test**: With two containers running, write a `.iris-dev.toml` pointing at container A, call `iris_execute`, then update file to container B, call `iris_execute` again — second call returns `$ZVersion` from container B.

### Tests for US1 (write first — must FAIL before implementation)

- [x] T020 [P] [US1] Write unit test: `check_reload()` with a config file whose mtime has changed but new connection is unreachable → `config_parse_error` is set, old connection preserved in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T021 [P] [US1] Write unit test: `check_reload()` when `config_watcher` is None → no-op, no panic in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T022 [US1] Write E2E test (`#[ignore]`): update `.iris-dev.toml` pointing at `iris-dev-iris`, verify first tool call works, update to point at unreachable container, verify next call returns `IRIS_UNREACHABLE` (not crash) in `tests/integration/test_live_reload_e2e.rs` (WRITE FIRST, must FAIL)

### TDD Gate

- [x] T023 [US1] **GATE**: Confirm T020–T022 all FAIL before implementation below

### Implementation for US1

- [x] T024 [US1] Wire `self.check_reload().await` at the top of every tool handler in `crates/iris-dev-core/src/tools/mod.rs` — add the call before any `let iris = self.get_iris()?` call in each `async fn` handler. Search for all `#[tool(` annotated methods and add the call at the start of the method body.
- [x] T025 [US1] Pass the resolved `.iris-dev.toml` path to `IrisTools` at construction in `crates/iris-dev-bin/src/cmd/mcp.rs` — after `apply_workspace_config()`, compute the config path via `workspace_root().join(".iris-dev.toml")`; if the file exists, build `ConfigWatcher` and pass to `with_registry_and_toolset()` via a new optional parameter or field setter
- [x] T026 [US1] **GATE-GREEN**: Run E2E T022 against `iris-dev-iris` — must pass

**Phase gate**: T022 E2E passes. Config file hot-reload works end-to-end.

---

## Phase 4: User Story 2 — Agent explicitly switches containers (Priority: P1)

**Goal**: `iris_select_container` stores the new connection after a successful probe. All subsequent tool calls use the new container. Fixes issue #11.

**Independent Test**: Call `iris_select_container(name="iris-dev-iris")`, then call `iris_execute(code="write $ZVersion,!")` — verify output matches `iris-dev-iris`.

### Tests for US2 (write first — must FAIL before implementation)

- [x] T027 [P] [US2] Write unit test: `iris_select_container` with unreachable container → existing connection preserved, error returned in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T028 [P] [US2] Write unit test: after simulated `iris_select_container` swap, `connection.lock().source == ConnectionSource::IrisSelectContainer` in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T029 [US2] Write E2E test (`#[ignore]`): call `iris_select_container(name="iris-dev-iris")`, then `check_config` → verify `connection_source: "iris_select_container"` and `container: "iris-dev-iris"` in `test_live_reload_e2e.rs` (WRITE FIRST, must FAIL)
- [x] T030 [US2] Write E2E test (`#[ignore]`): call `iris_select_container(name="iris-dev-iris")`, then `iris_execute(code="write $ZVersion,!")` → verify output is from iris-dev-iris (not previously connected container) in `test_live_reload_e2e.rs` (WRITE FIRST, must FAIL)

### TDD Gate

- [x] T031 [US2] **GATE**: Confirm T027–T030 FAIL before implementation

### Implementation for US2

- [x] T032 [US2] Rewrite `iris_select_container` handler in `crates/iris-dev-core/src/tools/mod.rs` — after successful probe, lock `self.connection`, replace with `ConnectionState::from_iris(new_conn, ConnectionSource::IrisSelectContainer, None)`, unlock; return response with `"switched": true`; on failure return error without modifying connection
- [x] T033 [US2] Remove the "restart required" note from `iris_select_container` response and tool description in `mod.rs`
- [x] T034 [US2] **GATE-GREEN**: Run E2E T029 and T030 — both must pass

**Phase gate**: T029 + T030 E2E tests pass. `iris_select_container` actually switches the active connection.

---

## Phase 5: User Story 3 — check_config tool (Priority: P2)

**Goal**: New `check_config` tool returns connection snapshot without any IRIS calls. Always succeeds.

**Independent Test**: Call `check_config` with no IRIS running — verify it returns `connected: false` and all 9 required fields, never returns `IRIS_UNREACHABLE`.

### Tests for US3 (write first — must FAIL before implementation)

- [x] T035 [P] [US3] Write unit test: `check_config` with `iris=None` → returns all 9 required fields, `connected: false`, no `IRIS_UNREACHABLE` error in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T035b [P] [US3] Write unit test: `check_config` with `config_watcher=None` (env-var-only session) → `config_file: null`, `connection_source: "env_vars"` (FR-008 coverage) in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T036 [P] [US3] Write unit test: all 4 `ConnectionSource` variants serialize to correct `connection_source` string values in `test_live_reload.rs` (WRITE FIRST, must FAIL)
- [x] T037 [US3] Write E2E test (`#[ignore]`): call `check_config` after session start → verify `connected: true`, `iris_version` populated, `write_tools_enabled: true`, `connection_source` one of valid values in `test_live_reload_e2e.rs` (WRITE FIRST, must FAIL)

### TDD Gate

- [x] T038 [US3] **GATE**: Confirm T035–T035b–T036–T037 all FAIL before implementation

### Implementation for US3

- [x] T039 [US3] Implement `check_config` tool handler in `crates/iris-dev-core/src/tools/mod.rs` — lock `self.connection`, read `ConnectionState`, build JSON response with all required fields (`connected`, `host`, `port`, `namespace`, `container`, `config_file`, `config_loaded_at` as ISO 8601, `iris_version`, `write_tools_enabled`, `connection_source`); do NOT call `get_iris()` or make any network calls; always return `ok_json(...)`
- [x] T040 [US3] Add `check_config` to all three toolset registration lists in `mod.rs` (Baseline, Nostub, Merged — it's read-only and has no docker dependency)
- [x] T041 [US3] **GATE-GREEN**: Run E2E T037 — must pass

**Phase gate**: T037 E2E passes. `check_config` returns accurate snapshot in all states.

---

## Phase 6: Polish & Cross-Cutting

- [x] T042 [P] Add `check_config` to README.md tool table — mark as `—` (no docker required), description: "Inspect active IRIS connection state — host, container, config file, last loaded, write tools status. Always succeeds."
- [x] T043 [P] Update `iris_list_containers` tool description to mention `iris_select_container` now actually switches the connection (remove restart caveat) in `crates/iris-dev-core/src/tools/mod.rs`
- [x] T044 [P] Run `cargo clippy --all-targets -- -D warnings` — must be clean
- [x] T045 [P] Run `cargo fmt --all -- --check` — must be clean
- [x] T046 [P] Run full unit test suite: `cargo test -p iris-dev-core` — all unit tests pass, no regressions
- [x] T047 [P] Run all E2E tests: `IRIS_HOST=localhost IRIS_WEB_PORT=52780 cargo test --test test_live_reload_e2e -- --ignored` — all 4 E2E tests pass

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately
- **Phase 2 (Foundational)**: Depends on Phase 1 — blocks all user story phases
- **Phase 3 (US1 hot-reload)**: Depends on Phase 2
- **Phase 4 (US2 iris_select_container)**: Depends on Phase 2; can run in parallel with Phase 3 after Phase 2 gate
- **Phase 5 (US3 check_config)**: Depends on Phase 2; also benefits from Phase 4 (uses ConnectionState)
- **Phase 6 (Polish)**: Depends on all phases complete

### Critical Path

```
T001-T010 (struct refactor) → T011-T019 (check_reload + ConnectionState)
                            → T020-T026 (US1 hot-reload)    ─┐
                            → T027-T034 (US2 select fix)    ─┤→ T042-T047 (polish)
                            → T035-T041 (US3 check_config)  ─┘
```

### Parallel Opportunities

**Phase 1**: T007, T008, T009 all touch different files — write concurrently with T001–T006.

**Phase 3+5 after Phase 2**: US1 and US3 can proceed in parallel:
- Developer A: Phase 3 (hot-reload wiring into tool handlers)
- Developer B: Phase 5 (check_config tool implementation)

**Phase 6**: All T042–T047 are independent — run simultaneously.

---

## Implementation Strategy

### MVP: Phases 1–4 (hot-reload + working iris_select_container)

1. Struct refactor — new types, constructor updates (Phase 1)
2. `check_reload()` with unit tests (Phase 2)
3. Wire `check_reload()` into all handlers (Phase 3) — most impactful change
4. Fix `iris_select_container` (Phase 4) — closes issue #11
5. **VALIDATE**: Config file change → next tool call uses new connection. `iris_select_container` actually switches.

### Full Feature

6. Phase 5: `check_config` tool (small, independent)
7. Phase 6: Polish, README, full suite
