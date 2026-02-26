import pytest
import thrift_rs_pyo3


def test_sum_as_string():
    assert thrift_rs_pyo3.sum_as_string(1, 1) == "2"
