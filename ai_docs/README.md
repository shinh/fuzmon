# fuzmon: Lightweight Fuzzy Process Monitor for Linux

## Overview

`fuzmon` is a lightweight, resident process monitoring tool designed for Linux systems.  
It periodically observes memory usage and collects stack traces from running processes (C/C++ and Python), with minimal overhead and zero external interference (no `gdb`).

---

## Goals

- Monitor memory usage (RSS, VSZ, Swap) of specified or auto-detected processes.
- Periodically attach to target processes using `ptrace` and retrieve stack traces:
  - C/C++: via `ptrace + libunwind-ptrace`
  - Python: via ptrace-compatible external tool (e.g., `py-spy`) or future native integration
- Run as a persistent background daemon with minimal CPU impact.
- Allow both direct PID-based targeting and config-based automatic process selection.
- Export structured logs in JSON or MessagePack format.

---

## Usage

```bash
fuzmon -p <pid>                 # Directly monitor a specific process by PID
fuzmon -c config.toml           # Use a config file to define selection/filter rules
```
