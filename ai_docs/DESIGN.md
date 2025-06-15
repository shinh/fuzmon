# Technical Design

## Process Monitoring

* Process info read from /proc/<pid>/status and /proc/<pid>/smaps or /statm
* Memory fields: RSS, VSZ, Swap
* Sampling interval defined by user (default: 60 seconds)

### Process Selection

- Mode 1: `-p <pid>` — monitor specific PID directly.
- Mode 2: `-c config.toml` — dynamic selection with filters:
  - Exclude rules via regex on process name
  - Include rules (e.g., user, memory usage)
  - **Automatically skip ptrace for "inactive" processes** based on:
    - CPU usage time (read from `/proc/<pid>/stat`)
    - Threshold: e.g., CPU usage percentage < N


### Stack Trace Collection

#### C/C++ Targets

- Mechanism:
  - `ptrace(PTRACE_ATTACH)` → wait for process stop
  - Use `libunwind-ptrace` to walk the stack
  - Resolve symbols via ELF/DWARF in target binary
- Notes:
  - Process is paused briefly (few ms expected)
  - Symbol resolution degrades on stripped binaries

#### Python Targets

- Mechanism:
  - Also attach via `ptrace`
  - From within process memory, find Python interpreter state
  - Call `PyEval_GetFrame()` or equivalent to extract stack
  - Inspired by `gdb`'s `py-bt` command (uses internal APIs)
- Advantages:
  - No external binary like `py-spy`
  - Compatible with embedded interpreters or stripped binaries (as long as libpython symbols are visible)
- Notes:
  - Requires interpreter introspection logic in fuzmon
  - Thread-aware implementation planned


### Output Format

- Log entry schema:

```json
{
  "timestamp": "2025-06-14T14:23:51Z",
  "pid": 12345,
  "process_name": "python3",
  "cpu_time_percent": 12.3,
  "memory": {
    "rss_kb": 20480,
    "vsz_kb": 105000,
    "swap_kb": 0
  },
  "stacktrace": [
    {
      "thread_id": 1,
      "frames": [
        {"function": "main", "file": "main.c", "line": 42},
        {"function": "do_work", "file": "worker.c", "line": 88}
      ]
    }
  ]
}
```

* Output formats:

** json: default, human-readable
** msgpack: optional, compact and efficient

* Configurable via config.toml

## Dependencies

* Rust ecosystem:

** nix for ptrace/syscall interface
** libunwind-sys or FFI bindings for libunwind-ptrace
** serde / rmp-serde for JSON/MessagePack output
* C/C++ symbols: target binary must have sufficient debug info for meaningful traces
* Python tracing: requires access to libpython symbols and memory layout

## Security and Privileges

* Requires CAP_SYS_PTRACE or root privileges
* /proc/sys/kernel/yama/ptrace_scope may need to be set to 0 on some systems
* Logs may contain sensitive memory/state information — log storage must be secured

