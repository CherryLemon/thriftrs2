import os

from thriftrs2 import load, serialize, deserialize, ProtocolType, loads, dumps


def test_compact():
    print("Testing CompactProtocol...")
    thrift_file = os.path.join(os.path.dirname(__file__), 'example.thrift')
    if not os.path.exists(thrift_file):
        with open(thrift_file, 'w') as f:
            f.write("""
struct User {
    1: i32 id
    2: string name
    3: string email
}
""")

    mod = load(thrift_file)
    user = mod.User(id=1, name="Compact User", email="compact@example.com")

    # Test Compact
    compact_blob = serialize(mod.User, user, proto=ProtocolType.Compact)
    print(f"Compact blob size: {len(compact_blob)}")

    back = deserialize(mod.User, compact_blob, proto=ProtocolType.Compact)
    print(f"Deserialized: {back}")
    assert back['id'] == 1
    assert back['name'] == "Compact User"

    # Test Binary (default)
    binary_blob = serialize(mod.User, user)
    print(f"Binary blob size: {len(binary_blob)}")
    assert len(binary_blob) > len(compact_blob) # Compact should be smaller

    back_bin = deserialize(mod.User, binary_blob)
    assert back_bin['id'] == 1
    print(dumps(mod.User, user))
    print("Multi-protocol test passed!")

if __name__ == "__main__":
    test_compact()

