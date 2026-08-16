# Everything SDK (Voidtools)

Vendored from Everything SDK zip for Window Hub Host IPC client.

- `dll/Everything64.dll` — runtime client (IPC to Everything.exe)
- `include/Everything.h` — API declarations
- `ipc/everything_ipc.h` — WM_COPYDATA protocol

License: MIT (see Everything.h header). Copyright (C) David Carpenter / voidtools.

The Host loads Everything64.dll at runtime; it does **not** require linking Everything.lib.
Everything search client must be running on the machine for queries to succeed.
