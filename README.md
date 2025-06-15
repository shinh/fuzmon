# fuzmon

Lightweight fuzzy process monitor for Linux.
Logs can be written in JSON (default) or MessagePack when `format = "msgpack"` is set in the config.
Python processes are traced using an embedded `py-spy` integration when possible.

```
fuzmon -o logs/             # write logs under ./logs
fuzmon -c config.toml       # use configuration file
# monitor a specific PID and write logs
fuzmon -p 1234 -o logs/
# logs default to /tmp/fuzmon when -o not specified
```

Each line in the log file is a JSON object similar to:

```json
{
  "timestamp": "2025-06-14T14:23:51Z",
  "pid": 12345,
  "process_name": "python3",
  "cpu_time_percent": 12.3,
  "memory": { "rss_kb": 20480, "vsz_kb": 105000, "swap_kb": 0 },
  "stacktrace": [[" 0: 0xdeadbeef main at main.c:42"]]
}
```
CPU usage is reported in the same way as the `top` command, so values can
exceed 100% when multiple threads are busy.
