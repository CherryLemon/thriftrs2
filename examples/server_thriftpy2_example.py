#!/usr/bin/env python3
"""
Thrift server example using thriftpy2.

This example starts a UserService server that handles:
  - get_user(user_id)  -> User
  - create_user(user)  -> bool
  - list_users()       -> list<User>

Run the server:
    python examples/server_example.py

Then test it from another terminal (or run client_example.py).
"""

import os
import time
import threading


import thriftpy2
from thriftpy2.transport import TCyBufferedTransportFactory
from thriftpy2.rpc import make_server

# ---------------------------------------------------------------------------
# Load the .thrift definition (same file used by test.py)
# ---------------------------------------------------------------------------
THRIFT_FILE = os.path.join(os.path.dirname(__file__), "example.thrift")
thrift_module = thriftpy2.load(THRIFT_FILE, module_name='example_thrift')

# ---------------------------------------------------------------------------
# In-memory "database"
# ---------------------------------------------------------------------------
_users: dict[int, dict] = {
    1: thrift_module.User(**{"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30}),
    2: thrift_module.User(**{"id": 2, "name": "Bob",   "email": "bob@example.com",   "age": 25}),
}
_next_id = 3


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
    uid = user.id
    if uid is None:
        uid = _next_id
        user.id = uid
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


class Handler:
    """Example handler class that dispatches to the above functions."""
    def get_user(self, user_id):
        return handle_get_user(user_id)

    def create_user(self, user):
        return handle_create_user(user)

    def list_users(self):
        return handle_list_users()


# ---------------------------------------------------------------------------
# Build and start the server
# ---------------------------------------------------------------------------

def main():

    service_def = thrift_module.UserService
    host, port = "127.0.0.1", 9090
    server = make_server(
        service_def,
        Handler(),
        host=host,
        port=port,
        trans_factory=TCyBufferedTransportFactory()
    )

    print(f"Starting UserService on {host}:{port}  (Ctrl-C to stop)")
    # serve() blocks and dispatches each connection in its own thread.
    server.serve()


if __name__ == "__main__":
    main()

