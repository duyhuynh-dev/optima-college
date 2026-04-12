from optima_ingest.catalog_detail import parse_prerequisite_groups


def test_parse_or_chain() -> None:
    g = parse_prerequisite_groups("Prerequisites: MATH120 OR MATH121 OR ECON102")
    assert g == [["MATH120", "MATH121", "ECON102"]]


def test_parse_and_with_or() -> None:
    g = parse_prerequisite_groups("Prerequisites: COMP112 AND MATH121 OR MATH122")
    assert g == [["COMP112"], ["MATH121", "MATH122"]]


def test_parse_none() -> None:
    assert parse_prerequisite_groups("Prerequisites: None") == []
    assert parse_prerequisite_groups("") == []
