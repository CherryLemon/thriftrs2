struct User {
    1: required i32 id;
    2: required string name;
    3: optional string email;
    4: optional i32 age;
}

struct Post {
    1: required i32 id;
    2: required string title;
    3: required string content;
    4: required i32 author_id;
    5: optional i64 created_at;
}

service UserService {
    User get_user(1: i32 user_id);
    bool create_user(1: User user);
    list<User> list_users();
}
