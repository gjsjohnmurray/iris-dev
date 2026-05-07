# Feature Specification: HTTP-Native Unit Test Runner

**Feature Branch**: `032-iris-test-http`  
**Created**: 2026-05-07  
**Status**: Draft  
**Closes**: #31

## Overview

The current `iris_test` tool requires `IRIS_CONTAINER` to be set (docker exec path) and immediately returns `DOCKER_REQUIRED` when it isn't. This means developers using IRIS without docker — or with a docker container that doesn't have exec accessible — have no way to run unit tests through iris-dev. The workaround discovered in practice (multi-step `iris_execute` → query `^UnitTestRoot` globals) works but requires many iterations and is fragile.

This feature enhances `iris_test` to run `%UnitTest.Manager` tests over pure Atelier REST (HTTP) and return structured, agent-readable results in a jUnit-compatible JSON format — no docker required. The docker exec path continues working as-is for environments where it's available.

---

## User Scenarios & Testing

### User Story 1 — Run tests without docker, get structured results (Priority: P1)

A developer using IRIS installed natively (no docker), or iris-dev pointed at a remote IRIS instance, asks the agent to run a unit test suite. The agent calls `iris_test` with a class pattern and immediately receives structured pass/fail results per test method — without needing to set `IRIS_CONTAINER`.

**Why this priority**: The entire issue #31 is about this case. It's the most common deployment model for production IRIS users and is blocked today. Without this, agents must use fragile multi-step workarounds.

**Independent Test**: Set `IRIS_CONTAINER` to empty, run `iris_test(pattern="MyApp.Tests")` against an IRIS instance with compiled test classes. Verify structured JSON results are returned in one call.

**Acceptance Scenarios**:

1. **Given** `IRIS_CONTAINER` is not set and IRIS has compiled `%UnitTest.TestCase` subclasses matching the pattern, **When** `iris_test(pattern="MyApp.Tests", namespace="USER")` is called, **Then** the response contains `{success, total, passed, failed, test_suites: [{name, tests: [{name, status, duration_ms, failure_message}]}]}` with accurate counts.
2. **Given** all tests pass, **When** `iris_test` is called, **Then** `success: true`, `failed: 0`, and each test entry has `status: "passed"`.
3. **Given** one or more tests fail, **When** `iris_test` is called, **Then** `success: false`, `failed: N`, and each failing test has `status: "failed"` with a non-empty `failure_message` containing the assertion that failed.
4. **Given** no test classes match the pattern, **When** `iris_test` is called, **Then** a clear error is returned: `error_code: "NO_TESTS_FOUND"` with the pattern that was searched.

---

### User Story 2 — Same tool works regardless of docker availability (Priority: P1)

An agent working on a project uses `iris_test` without knowing or caring whether `IRIS_CONTAINER` is set. The tool automatically uses the best available path: HTTP if no container is configured, docker exec if `IRIS_CONTAINER` is set.

**Why this priority**: Uniform interface — agents should not need to know or decide which execution path to use. Equals US1 in priority because it's the same call surface.

**Independent Test**: Run `iris_test` with `IRIS_CONTAINER` set; verify docker path is used. Run without `IRIS_CONTAINER`; verify HTTP path is used. Both return the same JSON shape.

**Acceptance Scenarios**:

1. **Given** `IRIS_CONTAINER` is set and the container is running, **When** `iris_test` is called, **Then** it uses the docker exec path and returns the same JSON shape as the HTTP path.
2. **Given** `IRIS_CONTAINER` is set but the container is not accessible, **When** `iris_test` is called, **Then** it falls back to the HTTP path automatically with `"path": "http_fallback"` in the response.
3. **Given** neither docker nor HTTP path can run tests, **When** `iris_test` is called, **Then** a clear error explains what is missing.

---

### User Story 3 — Agent can distinguish test failures from execution errors (Priority: P2)

An agent running tests can tell the difference between "tests ran but some failed" (normal development flow — fix the code) versus "test runner itself failed" (environment issue — fix infrastructure).

**Why this priority**: Agents take different follow-up actions based on this distinction. Without it, they may try to fix test code when the real problem is a missing namespace or permission error.

**Independent Test**: Run `iris_test` with a pattern that matches no classes → verify `NO_TESTS_FOUND`. Run with a class that errors in `%OnBeforeOneTest` → verify `status: "error"` distinct from `status: "failed"`.

**Acceptance Scenarios**:

1. **Given** a test class throws an unexpected error during execution (not an assertion failure), **When** `iris_test` is called, **Then** the affected test has `status: "error"` (distinct from `"failed"`) with the error text in `failure_message`.
2. **Given** the namespace doesn't exist, **When** `iris_test` is called, **Then** `error_code: "NAMESPACE_NOT_FOUND"` is returned immediately.
3. **Given** the jUnit file cannot be written (permissions issue), **When** `iris_test` is called, **Then** it falls back to reading `^UnitTestRoot` globals and adds `"source": "globals_fallback"` to the response.

---

### Edge Cases

- Pattern matches test classes in multiple packages — all should be included in results.
- Test class compiles but has no test methods (no methods starting with `Test`) — return `total: 0` with a warning.
- Very long test runs (>60 seconds) — default timeout applies; partial results not returned.
- IRIS version older than 2019.1 — `/junit` qualifier may not be available; falls back to `^UnitTestRoot` globals.
- Concurrent test runs from multiple agent sessions — each run uses a unique temp file path (UUID-keyed) to avoid collision.
- jUnit file contains non-UTF8 characters in test output — handled gracefully, problematic characters sanitized.

---

## Requirements

### Functional Requirements

- **FR-001**: When `IRIS_CONTAINER` is not set, `iris_test` MUST execute `%UnitTest.Manager.RunTest()` via the HTTP execution path (Atelier REST) rather than docker exec.
- **FR-002**: `iris_test` MUST pass the `/junit=<temp_path>` qualifier to `RunTest()` to capture results as jUnit XML in a unique temporary file path (UUID-keyed to avoid concurrent run collisions).
- **FR-003**: After execution, `iris_test` MUST read the jUnit XML file back, parse it, and return structured JSON in a single tool response.
- **FR-004**: The structured JSON response MUST include: `success` (bool), `total` (int), `passed` (int), `failed` (int), `errors` (int), `skipped` (int), `duration_ms` (float), `path` (enum: `"http"` | `"docker"` | `"http_fallback"`), `test_suites` (array).
- **FR-005**: Each test suite object MUST include: `name` (string), `tests` (int), `failures` (int), `errors` (int), `duration_ms` (float), `test_cases` (array).
- **FR-006**: Each test case object MUST include: `name` (string), `class_name` (string), `status` (enum: `"passed"` | `"failed"` | `"error"` | `"skipped"`), `duration_ms` (float), `failure_message` (string or null).
- **FR-007**: When `IRIS_CONTAINER` is set, `iris_test` MUST use the existing docker exec path. If docker exec fails, it MUST attempt the HTTP fallback and set `path: "http_fallback"`.
- **FR-008**: If the jUnit file cannot be read or parsed, `iris_test` MUST fall back to reading `^UnitTestRoot` globals and return the same JSON shape with `"source": "globals_fallback"`.
- **FR-009**: The temporary jUnit file MUST be deleted after reading, regardless of success or failure.
- **FR-010**: `iris_test` MUST accept a `timeout` parameter (default: 60 seconds) that bounds the test run duration.

### Key Entities

- **TestRun**: Top-level result — overall counts, path used, list of TestSuites.
- **TestSuite**: One `%UnitTest.TestCase` subclass — suite-level counts and list of TestCases.
- **TestCase**: One test method — name, status (`passed` | `failed` | `error` | `skipped`), duration, optional failure message.

---

## Success Criteria

### Measurable Outcomes

- **SC-001**: An agent can run a unit test suite against a non-docker IRIS instance in a single `iris_test` tool call with no retry loops or follow-up queries required.
- **SC-002**: The response JSON structure is identical whether the docker or HTTP path was used — agent code requires no branching on execution path.
- **SC-003**: Failed test cases include enough detail (assertion message) that an agent can identify and fix the failing code without additional tool calls in at least 80% of cases.
- **SC-004**: Test runs up to 50 test classes complete and return results within the configured timeout.
- **SC-E2E**: End-to-end test against a live IRIS instance with compiled `%UnitTest.TestCase` subclasses confirms correct pass/fail counts match what IRIS Management Portal shows for the same run.

---

## Assumptions

- Test classes are already compiled in the target IRIS namespace. Compilation is not part of this feature.
- `/junit` qualifier for `%UnitTest.Manager.RunTest()` is available on IRIS 2019.1+. For older versions, the `^UnitTestRoot` fallback path covers the gap.
- The IRIS instance has a writable `/tmp` directory accessible to the IRIS process (standard on all supported platforms). The temp file path uses `/tmp/iris_dev_junit_{uuid}.xml`.
- jUnit XML produced by IRIS's `%UnitTest.Manager` follows the standard Ant/JUnit XML schema.
- The HTTP execution path has sufficient output buffer for jUnit XML from typical test suites (hundreds of test cases). Very large suites (1000+ tests) may need chunked reads — deferred to a follow-on issue.
