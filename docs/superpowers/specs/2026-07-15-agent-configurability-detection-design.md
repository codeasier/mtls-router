# Agent Configurability Detection Design

## Problem

Agent configuration is incorrectly gated by whether the manager process can find an Agent CLI on its inherited `PATH`. Finder-launched macOS applications receive a restricted `PATH`, so supported Agents can appear unavailable even when their configuration exists. The same gate also prevents creating configuration before a CLI is installed or visible.

## Decision

`Detected` represents whether the manager supports configuring the Agent, not whether its CLI is visible. Claude Code, opencode, and Codex are all supported configuration targets and therefore always return `Detected: true`.

Executable lookup remains best-effort diagnostic metadata. When lookup succeeds, `Command` contains the executable path. When it fails, `Command` is empty. Configuration existence, validity, writability, and router configuration remain independent state fields.

The Codex-only home-directory detection fallback is removed because it is no longer needed and gave Agents inconsistent semantics.

## User Experience

- A missing configuration file is shown as ready to create and remains selectable.
- An existing valid configuration is shown as ready or configured and remains selectable when writable.
- Invalid or non-writable configuration remains blocked by existing safety checks.
- A CLI missing from the manager process `PATH` does not imply that the Agent is absent and does not block preview or write.

No IPC schema or desktop component changes are required because the existing UI already maps a detected, missing configuration to the ready-to-create state.

## Verification

Regression tests cover all supported Agents with CLI lookup failures and no configuration files. They assert that every Agent is detected, selectable by implication, and retains an empty command. Existing tests continue to cover valid, invalid, configured, missing, and non-writable configuration behavior. A service-level preview test verifies that missing CLI lookup no longer produces `AGENT_NOT_FOUND`.
