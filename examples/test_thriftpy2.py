import thriftpy2
from thriftpy2.utils import deserialize, serialize


def main():
    mod = thriftpy2.load('example.thrift', module_name='example_thrift')
    # Create user data
    user_data = {
        'id': 123,
        'name': 'John Doe',
        'email': 'john@example.com',
        'age': 30
    }

    # Get User struct definition
    User = mod.User

    # Serialize
    binary_data = serialize(User(**user_data))
    print(f"Serialized data: {binary_data.hex()}")

    # Deserialize
    deserialized = deserialize(User(), binary_data)
    print(f"Deserialized data: {deserialized}")

    # Verify round-trip
    assert deserialized.id == user_data['id']
    assert deserialized.name == user_data['name']
    assert deserialized.email == user_data['email']
    assert deserialized.age == user_data['age']

    print("✅ Round-trip serialization successful!")

if __name__ == '__main__':
    main()
