#!/usr/bin/env python3
"""
Thrift server example using thrift_rs_pyo3.

This example starts a UserService server that handles:
  - get_user(user_id)  -> User
  - create_user(user)  -> bool
  - list_users()       -> list<User>

Run the server:
    python examples/server_example.py

Then test it from another terminal (or run client_example.py).
"""

import os
import sys
import time
import threading

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'python'))

from thrift_rs_pyo3 import load, ThriftServer, TBufferedTransport

# ---------------------------------------------------------------------------
# In-memory "database"
# ---------------------------------------------------------------------------
_users: dict[int, dict] = {
    1: {"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30},
    2: {"id": 2, "name": "Bob",   "email": "bob@example.com",   "age": 25},
}
_next_id = 3

# ---------------------------------------------------------------------------
# Load the .thrift definition (same file used by test.py)
# ---------------------------------------------------------------------------
THRIFT_FILE = os.path.join(os.path.dirname(__file__), "example.thrift")
thrift_module = load(THRIFT_FILE)
service_def = thrift_module._parser.get_service("UserService")

# ---------------------------------------------------------------------------
# Handler functions
# Each handler receives **keyword arguments** matching the method's argument
# names and must return a value whose Python type matches the return type:
#   - struct   -> dict
#   - bool     -> bool
#   - list<T>  -> list
#   - void     -> None
# ---------------------------------------------------------------------------

def handle_get_user(user_id: int) -> dict | None:
    """Return the User dict for the given id, or None if not found."""
    print(f"  [server] get_user(user_id={user_id})")
    return _users.get(user_id)


def handle_create_user(user: dict) -> bool:
    """Insert a new user; returns True on success."""
    global _next_id
    uid = user.get("id", _next_id)
    if uid in _users:
        print(f"  [server] create_user -> already exists id={uid}")
        return False
    _users[uid] = user
    _next_id = max(_next_id, uid + 1)
    print(f"  [server] create_user -> created id={uid}")
    return True


def handle_list_users() -> list:
    """Return all users as a list of dicts."""
    print(f"  [server] list_users -> {len(_users)} users")
    return list(_users.values())


# ---------------------------------------------------------------------------
# Build and start the server
# ---------------------------------------------------------------------------

def main():
    server = ThriftServer(service_def, TBufferedTransport.transport_type)
    server.set_parser(thrift_module._parser)

    server.register_handler("get_user",    handle_get_user)
    server.register_handler("create_user", handle_create_user)
    server.register_handler("list_users",  handle_list_users)

    host, port = "127.0.0.1", 9090
    print(f"Starting UserService on {host}:{port}  (Ctrl-C to stop)")
    # serve() blocks and dispatches each connection in its own thread.
    server.serve(host, port)


if __name__ == "__main__":
    main()

