#!/usr/bin/env python3
"""
Thrift client example using thrift_rs_pyo3.

Connects to the UserService server started by server_example.py and makes
a few RPC calls to demonstrate the ThriftClient API.

Run the server first:
    python examples/server_example.py

Then run this script:
    python examples/client_example.py
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'python'))

from thrift_rs_pyo3 import load, make_client, TBufferedTransport, ThriftApplicationException

THRIFT_FILE = os.path.join(os.path.dirname(__file__), "example.thrift")


def main():
    # ── Load the .thrift definition ──────────────────────────────────────────
    thrift_module = load(THRIFT_FILE)
    service_def = thrift_module.UserService
    User = thrift_module.User

    # ── Connect to the server ────────────────────────────────────────────────
    # make_client() constructs a ThriftClient, calls open(), and returns it.
    # Pass `parser=` so nested struct types resolve correctly during (de)ser.
    with make_client(
        service_def,
        "127.0.0.1",
        9090,
        TBufferedTransport.transport_type,
    ) as client:

        # ── get_user(user_id=1) → User ───────────────────────────────────────
        user = client.call("get_user", user_id=1)
        print(f"get_user(1)      -> {user}")

        # ── create_user(user=...) → bool ─────────────────────────────────────
        new_user = User(id=99, name="Charlie", email="charlie@example.com", age=22)
        ok = client.call("create_user", user=new_user)
        print(f"create_user(99)  -> {ok}")

        # ── list_users() → list<User> ────────────────────────────────────────
        users = client.call("list_users")
        print(f"list_users()     -> {users}")

        # ── Exception handling ───────────────────────────────────────────────
        try:
            client.call("nonexistent_method")
        except ThriftApplicationException as e:
            print(f"Expected ThriftApplicationException: {e}")


if __name__ == "__main__":
    main()
