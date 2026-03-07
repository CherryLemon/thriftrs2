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
import asyncio
import os
from thrift_rs_pyo3 import load, ThriftServer, TBufferedTransport, make_server
# ---------------------------------------------------------------------------
# Load the .thrift definition (same file used by test.py)
# ---------------------------------------------------------------------------
THRIFT_FILE = os.path.join(os.path.dirname(__file__), "example.thrift")
thrift_module = load(THRIFT_FILE)
service_def = thrift_module.UserService
User = thrift_module.User
# ---------------------------------------------------------------------------
# In-memory "database"
# ---------------------------------------------------------------------------
_users: dict[int, dict] = {
    1: User(**{"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30}),
    2: User(**{"id": 2, "name": "Bob",   "email": "bob@example.com",   "age": 25}),
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

class Handler:
    def get_user(self, user_id: int) -> dict | None:
        """Return the User dict for the given id, or None if not found."""
        print(f"  [server] get_user(user_id={user_id})")
        return _users.get(user_id)


    def create_user(self, user: dict) -> bool:
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


    async def list_users(self) -> list:
        """Return all users as a list of dicts."""
        print(f"  [server] list_users -> {len(_users)} users")
        return list(_users.values())


# ---------------------------------------------------------------------------
# Build and start the server
# ---------------------------------------------------------------------------

async def main():
    host, port = "127.0.0.1", 9090
    server = make_server(
        service_def,
        Handler(),
        transport=TBufferedTransport.transport_type,
        workers=4
    )
    print(f"Starting UserService on {host}:{port}  (Ctrl-C to stop)")
    server.serve_forever(host, port)


if __name__ == "__main__":
    asyncio.run(main())

