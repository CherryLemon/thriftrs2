#!/usr/bin/env python3

import os

from thriftrs2 import load, serialize, deserialize


THRIFT_FILE = os.path.join(os.path.dirname(__file__), 'example.thrift')

def main():
    # Load thrift file
    thrift_module = load(THRIFT_FILE)

    print("Loaded structs:", thrift_module._parser.list_structs())

    # Create user data
    user_data = {
        'id': 123,
        'name': 'John Doe',
        'email': 'john@example.com',
        'age': 30
    }

    # Get User struct definition
    User = thrift_module.User
    print(User)
    # Serialize
    binary_data = serialize(User, user_data)
    print(f"Serialized data: {binary_data.hex()}")

    # Deserialize
    deserialized = deserialize(User, binary_data)
    print(f"Deserialized data: {deserialized}")

    # Verify round-trip
    assert deserialized['id'] == user_data['id']
    assert deserialized['name'] == user_data['name']
    assert deserialized['email'] == user_data['email']
    assert deserialized['age'] == user_data['age']

    print("✅ Round-trip serialization successful!")

if __name__ == '__main__':
    main()
