# ADR 0003: Fail-closed global hotkeys

Status: Accepted

## Context

Global hotkeys take a chord away from every other application. If two Snapline actions want the same chord, binding either one makes behaviour depend on registration order. If another application already owns a chord, Windows refuses it and the action silently does nothing.

## Decision

Snapline collects every chord of every enabled action and every enabled workflow, normalises modifiers and keys, and builds one plan. A chord with exactly one owner is bound. A chord with more than one owner is blocked and none of its owners is bound. Each bound chord is registered with `RegisterHotKey` and `MOD_NOREPEAT`. If Windows reports `ERROR_HOTKEY_ALREADY_REGISTERED`, that owner becomes blocked with an external reason. Blocked rows are shown with an error message, and one toast explains the count when the warning setting is on. Disabled workflows do not claim their chord. Replanning runs on settings changes that affect chords, on resume from sleep and when hotkeys are resumed.

## Consequences

The user is never left with an action that silently does nothing. Two configured actions that share a chord both stop working until one is changed. The default workflow takes its chord from the Capture region action rather than claiming a second binding. The design's Ctrl+Shift+R and Ctrl+Shift+T workflow defaults were changed by the plan to Ctrl+Alt+R and Ctrl+Alt+T so the default set is clash-free.
