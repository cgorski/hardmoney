"""The package surface: ``__all__``, the type stubs, and the runtime agree.

The stubs in ``hardmoney/_hardmoney.pyi`` are hand-written; this test is
what keeps them honest against the compiled module.
"""

from __future__ import annotations

import ast
import inspect
from pathlib import Path

import pytest

import hardmoney

STUB = Path(hardmoney.__file__).with_name("_hardmoney.pyi")


def _stub_module() -> ast.Module:
    return ast.parse(STUB.read_text(), filename=str(STUB))


def _stub_top_level_names(tree: ast.Module) -> set[str]:
    names: set[str] = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
    return names


def _stub_class_members(cls: ast.ClassDef) -> set[str]:
    members: set[str] = set()
    for node in cls.body:
        if isinstance(node, ast.FunctionDef):
            members.add(node.name)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            members.add(node.target.id)
    return members


def _runtime_public_members(cls: type) -> set[str]:
    return {name for name in vars(cls) if not name.startswith("_")}


def test_all_matches_the_stub_and_the_extension() -> None:
    tree = _stub_module()
    stub_names = _stub_top_level_names(tree)
    assert set(hardmoney.__all__) == stub_names
    from hardmoney import _hardmoney

    for name in hardmoney.__all__:
        assert getattr(hardmoney, name) is getattr(_hardmoney, name), name


@pytest.mark.parametrize(
    "class_name",
    ["Filing", "Line", "Validation", "Finding", "Reconciliation", "LineCheck"],
)
def test_class_members_match_the_stub(class_name: str) -> None:
    tree = _stub_module()
    [cls_node] = [
        n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == class_name
    ]
    stub_members = _stub_class_members(cls_node)
    runtime = getattr(hardmoney, class_name)
    assert runtime.__module__ == "hardmoney"
    # Every public runtime member is declared, and vice versa.
    assert {m for m in stub_members if not m.startswith("_")} == _runtime_public_members(
        runtime
    )
    # Every dunder the stub declares exists at runtime.
    for dunder in (m for m in stub_members if m.startswith("__")):
        assert hasattr(runtime, dunder), dunder
    # Properties in the stub are properties (getset descriptors) at runtime.
    for node in cls_node.body:
        if isinstance(node, ast.FunctionDef) and any(
            isinstance(d, ast.Name) and d.id == "property" for d in node.decorator_list
        ):
            assert inspect.isdatadescriptor(getattr(runtime, node.name)), node.name
    # Classes without a constructor cannot be instantiated from Python.
    with pytest.raises(TypeError):
        runtime()


def test_exceptions_hierarchy() -> None:
    assert issubclass(hardmoney.FecError, ValueError)
    assert issubclass(hardmoney.UnsupportedForm, hardmoney.FecError)
    assert hardmoney.FecError.__module__ == "hardmoney"
    assert hardmoney.FecError.__doc__ and "line_no" in hardmoney.FecError.__doc__
    assert hardmoney.UnsupportedForm("x").line_no is None


def test_docstrings_present() -> None:
    for name in hardmoney.__all__:
        obj = getattr(hardmoney, name)
        if callable(obj) or inspect.isclass(obj):
            assert obj.__doc__, name
    assert hardmoney.__doc__ and "FEC" in hardmoney.__doc__
