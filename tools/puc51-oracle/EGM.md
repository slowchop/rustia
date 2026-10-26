# PUC Lua 5.1.5 Oracle Helpers

## Overview

This directory contains shell helpers and a small C driver for the offline PUC Lua 5.1.5 compatibility oracle.

## Interface

- [`build.sh`](build.sh) compiles the pinned oracle sources into an ignored local build directory.
- [`capture-expectations.sh`](capture-expectations.sh), [`driver.c`](driver.c), and [`emit-rows.sh`](emit-rows.sh) capture fixture expectations and emit manifest/reference rows.

## Invariants

- `ci-bench` alone supplies Rustia timing evidence; oracle fixture generation and syntax checks do not ([repository policy](../../AGENTS.md#benchmark-policy)).
- The build helper requires the pinned Lua source tree and checks the expected stock-source adjustment before compiling.
- Emitted rows refer to checked-in `puc51-compat` fixtures and the pinned oracle identity ([`emit-rows.sh`](emit-rows.sh#L1)).

## Side Effects

- `build.sh` deletes and recreates `tools/puc51-oracle/build`; `emit-rows.sh` truncates and writes the two output paths supplied by its caller. Review those paths before running either command.

## Verification

Run from the repository root: `sh -n tools/puc51-oracle/build.sh` and `sh -n tools/puc51-oracle/emit-rows.sh` validate shell syntax without building the oracle or writing generated rows.
