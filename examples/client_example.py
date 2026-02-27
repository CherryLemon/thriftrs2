import thriftpy2
from thriftpy2.rpc import make_client
from thriftpy2.transport import TCyBufferedTransportFactory, TCyFramedTransportFactory

def main():
    # Load thrift file
    thrift_module = thriftpy2.load('example.thrift', module_name='example_thrift')

    # Create user data
    user_data = {
        'id': 124,
        'name': 'John Doe',
        'email': '',
        'age': 30
    }

    client = make_client(
        thrift_module.UserService,
        'localhost',
        9090,
        trans_factory=TCyBufferedTransportFactory()
    )
    r = client.create_user(thrift_module.User(**user_data))
    print(f"create_user -> {r}")
    print(client.list_users())


if __name__ == '__main__':
    main()
