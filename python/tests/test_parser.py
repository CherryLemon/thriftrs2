from __future__ import annotations

from io import StringIO

import pytest

import thriftrs2
from thriftrs2.loader import load_fp


def test_parser_starts_empty():
    parser = thriftrs2.ThriftParser()
    parser.parse("")
    assert parser.list_structs() == []
    assert parser.list_services() == []


def test_parse_single_struct_lists_name(thrift_files):
    parser = thriftrs2.ThriftParser()
    parser.parse(thrift_files.empty.read_text(encoding="utf-8"))
    assert parser.list_structs() == ["Empty"]


def test_get_struct_returns_none_for_missing(all_types_module):
    assert all_types_module._parser.get_struct("Missing") is None


def test_get_service_returns_none_for_missing(all_types_module):
    assert all_types_module._parser.get_service("Missing") is None


def test_load_sets_module_name_from_filename(thrift_files):
    module = thriftrs2.load(str(thrift_files.primitives))
    assert module.name == "primitives"


def test_load_uses_explicit_module_name(thrift_files):
    module = thriftrs2.load(str(thrift_files.primitives), module_name="custom_name")
    assert module.name == "custom_name"


def test_load_accepts_include_dirs_argument(thrift_files):
    module = thriftrs2.load(str(thrift_files.primitives), include_dirs=[str(thrift_files.primitives.parent)])
    assert module.PrimitiveValues.name == "PrimitiveValues"


def test_load_resolves_included_structs(tmp_path):
    include_dir = tmp_path / "idl"
    include_dir.mkdir()
    (include_dir / "common.thrift").write_text(
        "struct Shared { 1: required string name; }",
        encoding="utf-8",
    )
    main = tmp_path / "main.thrift"
    main.write_text(
        'include "common.thrift"\nstruct Holder { 1: required common.Shared value; }',
        encoding="utf-8",
    )

    module = thriftrs2.load(str(main), include_dirs=[str(include_dir)])

    instance = module.Holder(value={"name": "from-include"})
    assert instance.to_dict() == {"value": {"name": "from-include"}}


def test_extends_merges_struct_fields_and_service_methods():
    module = load_fp(
        StringIO(
            """
            struct Base { 1: optional string id = "base"; }
            struct Child extends Base { 2: required string name; }
            service BaseService { string ping(); }
            service ChildService extends BaseService { string echo(1: string value); }
            """
        ),
        "inline_extends",
    )

    assert module.Child(name="Ada").to_dict() == {"id": "base", "name": "Ada"}
    service = module.ChildService.service_def
    assert service.get_method("ping").name == "ping"
    assert service.get_method("echo").name == "echo"


def test_load_fp_loads_file_like_object():
    module = load_fp(StringIO("struct Item { 1: required string name; }"), "inline")
    assert module.name == "inline"
    assert module.Item.fields[0].name == "name"


def test_load_missing_file_raises_file_not_found():
    with pytest.raises(FileNotFoundError):
        thriftrs2.load("/does/not/exist.thrift")


def test_parse_required_and_optional_fields(all_types_module):
    fields = {field.name: field for field in all_types_module.AllTypes.fields}
    assert fields["flag"].required is True
    assert fields["note"].required is False


def test_field_name_can_start_with_optional_keyword_prefix():
    parser = thriftrs2.ThriftParser()
    parser.parse("struct Names { 1: optional string optional_note; 2: string required_label; }")
    struct_def = parser.get_struct("Names")
    assert [field.name for field in struct_def.fields] == ["optional_note", "required_label"]


def test_parse_struct_fields_with_comma_separators():
    parser = thriftrs2.ThriftParser()
    parser.parse(
        """
        struct Names {
            1: required i32 id,
            2: optional string name,
        }
        """
    )
    struct_def = parser.get_struct("Names")
    assert [field.name for field in struct_def.fields] == ["id", "name"]


def test_parse_default_value_without_exposing_it(all_types_module):
    child_fields = {field.name: field for field in all_types_module.Child.fields}
    assert child_fields["name"].name == "name"
    assert child_fields["name"].required is True


def test_parse_nested_container_fields(containers_module):
    names = [field.name for field in containers_module.ContainerValues.fields]
    assert names == ["numbers", "tags", "counters", "addresses", "numeric_names", "grouped"]


def test_parse_service_lists_name(all_types_module):
    assert all_types_module._parser.list_services() == ["FixtureService"]


def test_service_attribute_is_lazy_wrapper(all_types_module):
    service = all_types_module.FixtureService
    assert "FixtureService" in repr(service)
    assert service.service_def.name == "FixtureService"


def test_service_get_method_returns_method(all_types_module):
    method = all_types_module.FixtureService.service_def.get_method("echo")
    assert method.name == "echo"
    assert method.oneway is False
    assert [field.name for field in method.arguments] == ["value"]


def test_parse_oneway_method(all_types_module):
    method = all_types_module.FixtureService.service_def.get_method("notify")
    assert method.oneway is True
    assert method.arguments[0].name == "message"


def test_parse_void_method_as_supported_placeholder(all_types_module):
    method = all_types_module.FixtureService.service_def.get_method("ping")
    assert method.name == "ping"
    assert "Struct(\"void\")" in repr(method)


def test_parse_comments_and_unknown_top_level_tokens():
    parser = thriftrs2.ThriftParser()
    parser.parse(
        """
        namespace py thrift.example
        include "other.thrift"
        // service data below
        struct Item { 1: required i32 id; }
        """
    )
    assert parser.list_structs() == ["Item"]
    assert parser.list_includes() == ["other.thrift"]
    assert parser.namespaces() == {"py": "thrift.example"}


def test_parse_typedef_enum_const_exception_and_throws():
    parser = thriftrs2.ThriftParser()
    parser.parse(
        """
        typedef i64 Timestamp
        enum Status { OK = 1, FAILED = 2 }
        const map<string, string> DEFAULT_LABELS = {"source": "test"}
        exception NotFound { 1: string message; }
        struct Event {
            1: required Timestamp created_at;
            2: optional Status status = Status.OK;
            3: optional list<i32> ids = [1, 2, 3];
        }
        service Events {
            Event get(1: required i32 id) throws (1: NotFound missing);
        }
        """
    )

    assert parser.list_enums() == ["Status"]
    assert parser.get_enum("Status") == {"OK": 1, "FAILED": 2}
    event_fields = {field.name: field for field in parser.get_struct("Event").fields}
    assert "I64" in repr(event_fields["created_at"])
    assert "I32" in repr(event_fields["status"])
    assert "List" in repr(event_fields["ids"])
    method = parser.get_service("Events").get_method("get")
    assert method.exceptions[0].name == "missing"


def test_module_exposes_enum_union_annotations_and_defaults():
    module = load_fp(
        StringIO(
            """
            enum Status { OK = 1, FAILED = 2 }
            union Choice (scope="test") {
                1: optional Status status = Status.OK;
                2: optional string label = "fallback" (ui.hidden="true");
                3: optional list<i32> ids = [1, 2, 3];
            }
            """
        ),
        "inline_defaults",
    )

    assert module.Status.OK == 1
    assert module.Status["FAILED"] == 2
    instance = module.Choice()
    assert instance.to_dict() == {"status": 1, "label": "fallback", "ids": [1, 2, 3]}

    encoded = thriftrs2.serialize(module.Choice, {}, proto=thriftrs2.ProtocolType.JSON)
    decoded = thriftrs2.deserialize(module.Choice, encoded, proto=thriftrs2.ProtocolType.JSON)
    assert decoded == {"status": 1, "label": "fallback", "ids": [1, 2, 3]}


def test_parse_invalid_field_id_raises_value_error():
    parser = thriftrs2.ThriftParser()
    with pytest.raises(ValueError):
        parser.parse("struct Bad { id: required string name; }")


def test_parse_unclosed_struct_raises_value_error():
    parser = thriftrs2.ThriftParser()
    with pytest.raises(ValueError):
        parser.parse("struct Bad { 1: required string name;")


def test_module_missing_attribute_raises_attribute_error(all_types_module):
    with pytest.raises(AttributeError):
        _ = all_types_module.DoesNotExist
