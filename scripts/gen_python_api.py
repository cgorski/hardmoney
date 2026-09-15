#!/usr/bin/env python3
"""Generate the Python API reference chapter of the book from the type stub.

``python/python/hardmoney/_hardmoney.pyi`` is the single source of truth for
the Python package's public surface: every signature, annotation, and
docstring. This script parses it with :mod:`ast` (no import of the compiled
extension, so it needs no Rust toolchain) and writes
``book/src/python-api.md``: one ``##`` section per class, one ``###`` entry
per member with its full signature and docstring, properties marked with
``@property``, and hardmoney types cross-linked within the page.

The output is deterministic: the same stub always produces the same
Markdown, byte for byte, so the generated chapter is committed and CI
fails when it is stale.

Usage (from the repository root; only the standard library is needed)::

    python3 scripts/gen_python_api.py                 # write book/src/python-api.md
    python3 scripts/gen_python_api.py --check         # exit 1 if the committed file is stale
    python3 scripts/gen_python_api.py --output PATH   # write somewhere else (or - for stdout)

Two further modes compare the stub with the compiled module and therefore
need the package importable (build it into a virtualenv first, e.g.
``cd python && maturin develop --release``)::

    python3 scripts/gen_python_api.py --check-stub    # report members the module documents but the stub does not
    python3 scripts/gen_python_api.py --sync-stub     # copy those docstrings into the stub

``--check-stub`` ignores CPython's generic slot text ("Return repr(self).")
so that dunders without a stub docstring are not reported as gaps.

Docstring conventions the converter understands: RST double-backtick
literals become Markdown code spans; ``:class:``, ``:func:``, ``:meth:``,
``:attr:`` and ``:data:`` roles become links to the matching entry when it
exists on the page (unqualified ``:meth:`` / ``:attr:`` targets resolve
inside the current class) and plain code spans otherwise.
"""

from __future__ import annotations

import argparse
import ast
import difflib
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Optional

ROOT = Path(__file__).resolve().parents[1]
STUB = ROOT / "python" / "python" / "hardmoney" / "_hardmoney.pyi"
OUTPUT = ROOT / "book" / "src" / "python-api.md"

# The order sections appear in. Names not listed here are appended after
# the listed ones, in stub order, so a new export is never silently
# dropped from the reference.
FUNCTION_ORDER = ["parse", "parse_file", "fetch", "tables", "layout", "field_spec"]
CONSTANT_ORDER = ["BUNDLED_SPEC_VERSION", "__version__"]
CLASS_ORDER = [
    "Filing",
    "Line",
    "Validation",
    "Finding",
    "Reconciliation",
    "LineCheck",
    "FecError",
    "UnsupportedForm",
]

BANNER = (
    "<!-- Generated from python/python/hardmoney/_hardmoney.pyi by "
    "scripts/gen_python_api.py; do not edit. -->"
)

INTRO = """\
> Generated from
> [`_hardmoney.pyi`](https://github.com/cgorski/hardmoney/blob/main/python/python/hardmoney/_hardmoney.pyi)
> by `scripts/gen_python_api.py`; do not edit this page by hand. Edit the
> stub and run `python3 scripts/gen_python_api.py`.

Every name below is importable from the top-level package:
`import hardmoney`. The same text is available at the prompt as
`help(hardmoney.Filing)`, `help(hardmoney.parse)`, and so on, because
the compiled module carries the same docstrings as the stub. For a guided
introduction see [Getting started with Python](./python.md); for worked
examples see the [Python cookbook](./python-cookbook.md).

Types in signatures are written the way Python 3.10+ prints them
(`int | None` rather than `Optional[int]`); `Decimal` is
`decimal.Decimal` and `date` is `datetime.date`.
"""


# ---------------------------------------------------------------------------
# Stub model
# ---------------------------------------------------------------------------


@dataclass
class Member:
    """One function, method, property, or annotated attribute."""

    name: str
    node: ast.AST
    kind: str  # "function" | "method" | "property" | "attribute"
    doc: Optional[str]
    overloads: list[ast.FunctionDef] = field(default_factory=list)


@dataclass
class Klass:
    name: str
    node: ast.ClassDef
    doc: Optional[str]
    bases: list[str]
    decorators: list[str]
    members: list[Member]


@dataclass
class Stub:
    functions: list[Member]
    constants: list[Member]
    classes: list[Klass]

    @property
    def class_names(self) -> set[str]:
        return {c.name for c in self.classes}

    def member_names(self, cls: str) -> set[str]:
        for c in self.classes:
            if c.name == cls:
                return {m.name for m in c.members}
        return set()


def _decorator_names(node: ast.AST) -> list[str]:
    out = []
    for d in getattr(node, "decorator_list", []):
        out.append(ast.unparse(d))
    return out


def _attribute_docstring(body: list[ast.stmt], index: int) -> Optional[str]:
    """The string literal directly after ``body[index]``, if any (PEP 257's
    attribute docstring convention)."""
    if index + 1 < len(body):
        nxt = body[index + 1]
        if (
            isinstance(nxt, ast.Expr)
            and isinstance(nxt.value, ast.Constant)
            and isinstance(nxt.value.value, str)
        ):
            return ast.get_docstring(ast.Module(body=[nxt], type_ignores=[]))
    return None


def _collect_members(body: list[ast.stmt], scope: str) -> list[Member]:
    """Functions/attributes in a module or class body, overloads folded into
    one entry. ``scope`` is "function" or "method"."""
    members: list[Member] = []
    by_name: dict[str, Member] = {}
    for i, node in enumerate(body):
        if isinstance(node, ast.FunctionDef):
            decorators = _decorator_names(node)
            if "overload" in decorators:
                m = by_name.get(node.name)
                if m is None:
                    m = Member(node.name, node, scope, None)
                    by_name[node.name] = m
                    members.append(m)
                m.overloads.append(node)
                continue
            kind = "property" if "property" in decorators else scope
            doc = ast.get_docstring(node)
            m = by_name.get(node.name)
            if m is None:
                m = Member(node.name, node, kind, doc)
                by_name[node.name] = m
                members.append(m)
            else:
                m.node, m.kind, m.doc = node, kind, doc
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            members.append(
                Member(node.target.id, node, "attribute", _attribute_docstring(body, i))
            )
    return members


def load_stub(path: Path = STUB) -> Stub:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    functions: list[Member] = []
    constants: list[Member] = []
    classes: list[Klass] = []
    for m in _collect_members(tree.body, "function"):
        (constants if m.kind == "attribute" else functions).append(m)
    for node in tree.body:
        if isinstance(node, ast.ClassDef):
            classes.append(
                Klass(
                    name=node.name,
                    node=node,
                    doc=ast.get_docstring(node),
                    bases=[ast.unparse(b) for b in node.bases],
                    decorators=_decorator_names(node),
                    members=_collect_members(node.body, "method"),
                )
            )
    return Stub(functions, constants, classes)


def _ordered(items: list, order: list[str]):
    rank = {name: i for i, name in enumerate(order)}
    return sorted(items, key=lambda it: (rank.get(it.name, len(order)), 0))


# ---------------------------------------------------------------------------
# Signatures
# ---------------------------------------------------------------------------


class _Simplify(ast.NodeTransformer):
    """``Optional[X]`` -> ``X | None``, ``Union[A, B]`` -> ``A | B``,
    ``decimal.Decimal`` -> ``Decimal``, ``datetime.date`` -> ``date``."""

    def visit_Subscript(self, node: ast.Subscript) -> ast.AST:
        node = self.generic_visit(node)  # type: ignore[assignment]
        if isinstance(node.value, ast.Name):
            if node.value.id == "Optional":
                return ast.BinOp(node.slice, ast.BitOr(), ast.Constant(None))
            if node.value.id == "Union" and isinstance(node.slice, ast.Tuple):
                elts = node.slice.elts
                out: ast.expr = elts[0]
                for e in elts[1:]:
                    out = ast.BinOp(out, ast.BitOr(), e)
                return out
        return node

    def visit_Attribute(self, node: ast.Attribute) -> ast.AST:
        if isinstance(node.value, ast.Name):
            if (node.value.id, node.attr) in {("decimal", "Decimal"), ("datetime", "date")}:
                return ast.Name(node.attr, ast.Load())
        return self.generic_visit(node)


def render_annotation(node: Optional[ast.expr]) -> str:
    if node is None:
        return ""
    import copy

    simplified = _Simplify().visit(copy.deepcopy(node))
    return ast.unparse(simplified)


def render_signature(fn: ast.FunctionDef) -> str:
    a = fn.args
    parts: list[str] = []

    def one(arg: ast.arg, default: Optional[ast.expr]) -> str:
        s = arg.arg
        if arg.annotation is not None:
            s += f": {render_annotation(arg.annotation)}"
        if default is not None:
            s += f" = {ast.unparse(default)}" if arg.annotation else f"={ast.unparse(default)}"
        return s

    positional = a.posonlyargs + a.args
    defaults: list[Optional[ast.expr]] = [None] * (len(positional) - len(a.defaults)) + list(
        a.defaults
    )
    for i, arg in enumerate(positional):
        parts.append(one(arg, defaults[i]))
        if a.posonlyargs and i == len(a.posonlyargs) - 1:
            parts.append("/")
    if a.vararg is not None:
        parts.append("*" + one(a.vararg, None))
    elif a.kwonlyargs:
        parts.append("*")
    for arg, default in zip(a.kwonlyargs, a.kw_defaults):
        parts.append(one(arg, default))
    if a.kwarg is not None:
        parts.append("**" + one(a.kwarg, None))
    sig = f"def {fn.name}({', '.join(parts)})"
    if fn.returns is not None:
        sig += f" -> {render_annotation(fn.returns)}"
    return sig


def annotation_names(node: Optional[ast.expr]) -> set[str]:
    """Bare identifiers appearing in an annotation."""
    if node is None:
        return set()
    return {n.id for n in ast.walk(node) if isinstance(n, ast.Name)}


def signature_names(fn: ast.FunctionDef) -> set[str]:
    names: set[str] = set()
    a = fn.args
    for arg in a.posonlyargs + a.args + a.kwonlyargs + [a.vararg, a.kwarg]:
        if arg is not None:
            names |= annotation_names(arg.annotation)
    names |= annotation_names(fn.returns)
    return names


# ---------------------------------------------------------------------------
# Anchors and docstring conversion
# ---------------------------------------------------------------------------


def anchor(*parts: str) -> str:
    """A stable heading id: ``anchor("Filing", "lines_for")`` is
    ``filing-lines_for``. Leading underscores are dropped so
    ``__version__`` becomes ``version``."""
    return "-".join(p.lower().strip("_") for p in parts)


_LITERAL = re.compile(r"``(.+?)``")
_ROLE = re.compile(r":(class|func|meth|attr|data|exc|mod):`([^`]+)`")


def convert_docstring(doc: Optional[str], stub: Stub, cls: Optional[str] = None) -> str:
    """RST-flavoured stub docstring to Markdown with in-page links."""
    if not doc:
        return "*No documentation.*"

    def role(m: re.Match[str]) -> str:
        kind, target = m.group(1), m.group(2)
        display = target
        link = None
        if kind == "class" or kind == "exc":
            if target in stub.class_names:
                link = f"#{anchor(target)}"
        elif kind == "func":
            if target in {f.name for f in stub.functions}:
                link = f"#{anchor(target)}"
        elif kind == "data":
            if target in {c.name for c in stub.constants}:
                link = f"#{anchor(target)}"
        elif kind in ("meth", "attr"):
            if "." in target:
                owner, member = target.rsplit(".", 1)
            else:
                owner, member = cls or "", target
            if owner in stub.class_names and member in stub.member_names(owner):
                link = f"#{anchor(owner, member)}"
        code = f"`{display}`"
        return f"[{code}]({link})" if link else code

    text = _ROLE.sub(role, doc)
    text = _LITERAL.sub(r"`\1`", text)
    return text.strip()


def related_links(
    names: Iterable[str], stub: Stub, exclude: str = "", already_linked: str = ""
) -> str:
    """A "See also" line linking every hardmoney class named in a signature
    that the docstring text (``already_linked``) does not link itself."""
    names = set(names)

    def wanted(n: str) -> bool:
        return n in stub.class_names and n != exclude and f"(#{anchor(n)})" not in already_linked

    hits = [n for n in CLASS_ORDER if n in names and wanted(n)]
    hits += sorted(n for n in names if wanted(n) and n not in hits)
    if not hits:
        return ""
    return "See also: " + ", ".join(f"[`{n}`](#{anchor(n)})" for n in hits) + "."


# ---------------------------------------------------------------------------
# Markdown
# ---------------------------------------------------------------------------


def _code(lines: Iterable[str]) -> list[str]:
    return ["```python", *lines, "```"]


def render_member(m: Member, stub: Stub, cls: Optional[str]) -> list[str]:
    out: list[str] = []
    ident = anchor(cls, m.name) if cls else anchor(m.name)
    # Backticks keep `__getitem__` from rendering as bold "getitem".
    out.append(f"### `{m.name}` {{#{ident}}}")
    out.append("")
    names: set[str] = set()
    if m.kind == "attribute":
        node = m.node
        assert isinstance(node, ast.AnnAssign)
        out += _code([f"{m.name}: {render_annotation(node.annotation)}"])
        names = annotation_names(node.annotation)
    else:
        block: list[str] = []
        node = m.node
        assert isinstance(node, ast.FunctionDef)
        if m.overloads:
            # The overloads are the signature type checkers see; the
            # untyped implementation line below them adds nothing.
            for ov in m.overloads:
                block.append("@overload")
                block.append(render_signature(ov))
                names |= signature_names(ov)
        else:
            if m.kind == "property":
                block.append("@property")
            block.append(render_signature(node))
            names |= signature_names(node)
        out += _code(block)
    out.append("")
    if m.kind == "property":
        out.append("*Read-only property.*")
        out.append("")
    doc = m.doc
    if doc is None and m.overloads:
        for ov in m.overloads:
            doc = ast.get_docstring(ov)
            if doc:
                break
    body = convert_docstring(doc, stub, cls)
    out.append(body)
    see = related_links(names, stub, exclude=cls or "", already_linked=body)
    if see:
        out.append("")
        out.append(see)
    out.append("")
    return out


def render_class(k: Klass, stub: Stub) -> list[str]:
    out: list[str] = [f"## {k.name} {{#{anchor(k.name)}}}", ""]
    head = []
    for d in k.decorators:
        head.append(f"@{d}")
    bases = f"({', '.join(k.bases)})" if k.bases else ""
    head.append(f"class {k.name}{bases}")
    out += _code(head)
    out.append("")
    body = convert_docstring(k.doc, stub, k.name)
    out.append(body)
    base_names = {b for b in k.bases if b in stub.class_names}
    see = related_links(base_names, stub, exclude=k.name, already_linked=body)
    if see:
        out.append("")
        out.append(see)
    out.append("")
    props = [m for m in k.members if m.kind == "property"]
    attrs = [m for m in k.members if m.kind == "attribute"]
    methods = [m for m in k.members if m.kind == "method"]
    for group in (attrs, props, methods):
        for m in group:
            out += render_member(m, stub, k.name)
    return out


def render(stub: Stub) -> str:
    functions = _ordered(stub.functions, FUNCTION_ORDER)
    constants = _ordered(stub.constants, CONSTANT_ORDER)
    classes = _ordered(stub.classes, CLASS_ORDER)

    out: list[str] = [BANNER, "", "# Python API reference", "", INTRO]
    out.append("Contents:")
    out.append("")
    if functions:
        out.append("- [Functions](#functions)")
    if constants:
        out.append("- [Constants](#constants)")
    for k in classes:
        out.append(f"- [`{k.name}`](#{anchor(k.name)})")
    out.append("")

    if functions:
        out += ["## Functions {#functions}", ""]
        for m in functions:
            out += render_member(m, stub, None)
    if constants:
        out += ["## Constants {#constants}", ""]
        for m in constants:
            out += render_member(m, stub, None)
    for k in classes:
        out += render_class(k, stub)

    text = "\n".join(out).rstrip("\n") + "\n"
    # Collapse runs of blank lines left by optional sections.
    return re.sub(r"\n{3,}", "\n\n", text)


# ---------------------------------------------------------------------------
# Stub vs. compiled module
# ---------------------------------------------------------------------------


def _generic_slot_docs() -> set[str]:
    docs: set[str] = set()
    for t in (object, list, dict, tuple, str, int, BaseException):
        for name in dir(t):
            if name.startswith("__"):
                d = getattr(getattr(t, name, None), "__doc__", None)
                if isinstance(d, str):
                    docs.add(d)
    return docs


def stub_gaps(stub: Stub) -> list[tuple[str, ast.AST, str]]:
    """``(dotted name, stub node, runtime docstring)`` for every public
    class or member the compiled module documents and the stub does not."""
    try:
        import hardmoney  # type: ignore[import-not-found]
    except ImportError as e:  # pragma: no cover - depends on the environment
        sys.exit(
            f"error: cannot import hardmoney ({e}); build it first, e.g. "
            "`cd python && maturin develop --release`"
        )
    generic = _generic_slot_docs()
    gaps: list[tuple[str, ast.AST, str]] = []

    def runtime_doc(obj: object) -> Optional[str]:
        d = getattr(obj, "__doc__", None)
        if not isinstance(d, str) or not d.strip() or d in generic:
            return None
        return d

    for m in stub.functions:
        if m.doc is None:
            rt = getattr(hardmoney, m.name, None)
            d = runtime_doc(rt) if rt is not None else None
            if d:
                gaps.append((m.name, m.node, d))
    for k in stub.classes:
        rt_cls = getattr(hardmoney, k.name, None)
        if rt_cls is None:
            continue
        if k.doc is None and runtime_doc(rt_cls):
            gaps.append((k.name, k.node, runtime_doc(rt_cls) or ""))
        for m in k.members:
            if m.doc is not None or m.kind == "attribute":
                continue
            attr = getattr(rt_cls, m.name, None)
            d = runtime_doc(attr) if attr is not None else None
            if d:
                gaps.append((f"{k.name}.{m.name}", m.node, d))
    return gaps


def sync_stub(stub: Stub, gaps: list[tuple[str, ast.AST, str]]) -> int:
    """Insert the runtime docstring into every stub member whose body is a
    bare ``...``. Returns the number of edits made."""
    src = STUB.read_text(encoding="utf-8")
    lines = src.splitlines(keepends=True)
    edits = 0
    # Bottom-up so earlier line numbers stay valid.
    for name, node, doc in sorted(gaps, key=lambda g: -g[1].lineno):
        body = getattr(node, "body", None)
        if not (
            body
            and len(body) == 1
            and isinstance(body[0], ast.Expr)
            and isinstance(body[0].value, ast.Constant)
            and body[0].value.value is Ellipsis
        ):
            print(f"skip {name}: body is not a bare `...`; add the docstring by hand", file=sys.stderr)
            continue
        ell = body[0]
        indent = " " * (node.col_offset + 4)
        doc_lines = doc.strip().splitlines()
        if len(doc_lines) == 1:
            rendered = f'{indent}"""{doc_lines[0]}"""\n'
        else:
            rendered = f'{indent}"""{doc_lines[0]}\n' + "".join(
                f"{indent}{ln}\n" if ln.strip() else "\n" for ln in doc_lines[1:]
            ) + f'{indent}"""\n'
        line = lines[ell.lineno - 1]
        if ell.lineno == node.lineno:
            # `def f(self) -> str: ...` on one line: keep the header, drop `...`.
            head = line[: ell.col_offset].rstrip()
            lines[ell.lineno - 1] = head + "\n" + rendered
        else:
            lines[ell.lineno - 1] = rendered
        edits += 1
        print(f"added docstring to {name}", file=sys.stderr)
    if edits:
        STUB.write_text("".join(lines), encoding="utf-8")
    return edits


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main(argv: Optional[list[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--stub", type=Path, default=STUB, help=f"stub to read (default {STUB.relative_to(ROOT)})")
    p.add_argument("--output", type=Path, default=OUTPUT, help=f"where to write (default {OUTPUT.relative_to(ROOT)}; - for stdout)")
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="exit 1 if --output differs from what would be generated")
    mode.add_argument("--check-stub", action="store_true", help="exit 1 if the compiled module documents something the stub does not")
    mode.add_argument("--sync-stub", action="store_true", help="copy missing docstrings from the compiled module into the stub")
    args = p.parse_args(argv)

    stub = load_stub(args.stub)

    if args.check_stub or args.sync_stub:
        gaps = stub_gaps(stub)
        if not gaps:
            print("stub docstrings are complete relative to the compiled module")
            return 0
        if args.check_stub:
            for name, _, doc in gaps:
                first = doc.strip().splitlines()[0]
                print(f"{name}: stub has no docstring; module says: {first}")
            return 1
        sync_stub(stub, gaps)
        return 0

    text = render(stub)
    if args.check:
        try:
            current = args.output.read_text(encoding="utf-8")
        except FileNotFoundError:
            print(f"{args.output} does not exist; run scripts/gen_python_api.py", file=sys.stderr)
            return 1
        if current == text:
            print(f"{args.output.relative_to(ROOT) if args.output.is_absolute() else args.output} is up to date")
            return 0
        sys.stdout.writelines(
            difflib.unified_diff(
                current.splitlines(keepends=True),
                text.splitlines(keepends=True),
                fromfile=str(args.output),
                tofile="generated",
            )
        )
        print(f"\n{args.output} is stale; run scripts/gen_python_api.py", file=sys.stderr)
        return 1

    if str(args.output) == "-":
        sys.stdout.write(text)
    else:
        args.output.write_text(text, encoding="utf-8")
        print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
