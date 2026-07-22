#!/usr/bin/env python3
"""Verify a wasm-bindgen module's externref table is intact after wasm-opt.

wasm-bindgen's reference-types output (the default since Rust 1.82) exports the
externref heap as `__wbindgen_externrefs` and grows it by four at start-up in
`__wbindgen_init_externref_table`.  Some binaryen builds — notably the ancient
one in Ubuntu's `apt` — reorder a module's tables while optimising but leave
that export bound to its old index, which is now the fixed-size *function*
table.  The browser then throws

    RangeError: WebAssembly.Table.grow(): failed to grow table by 4

on the very first frame and the page never starts.

This checks the actual invariant the runtime depends on: the
`__wbindgen_externrefs` export must point at an `externref` table that is still
growable (no fixed maximum).  Exit 0 if so — or if the export is absent, i.e. a
module built without reference types, which has nothing to grow.  Exit 1 on a
mismatch so the build can fall back to the un-optimised wasm.
"""
import sys

EXTERNREF = 0x6F
FUNCREF = 0x70


def _uleb(data, off):
    result = shift = 0
    while True:
        byte = data[off]
        off += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, off
        shift += 7


def inspect(path):
    data = open(path, "rb").read()
    if data[:4] != b"\0asm":
        raise ValueError("not a wasm module")
    pos = 8
    tables = []  # (elem_type, has_max)
    export_idx = None  # table index exported as __wbindgen_externrefs
    while pos < len(data):
        sec_id = data[pos]
        pos += 1
        size, pos = _uleb(data, pos)
        end = pos + size
        if sec_id == 4:  # table section
            count, q = _uleb(data, pos)
            for _ in range(count):
                elem_type = data[q]
                q += 1
                flags = data[q]
                q += 1
                _min, q = _uleb(data, q)
                has_max = bool(flags & 1)
                if has_max:
                    _max, q = _uleb(data, q)
                tables.append((elem_type, has_max))
        elif sec_id == 7:  # export section
            count, q = _uleb(data, pos)
            for _ in range(count):
                name_len, q = _uleb(data, q)
                name = data[q:q + name_len].decode()
                q += name_len
                kind = data[q]
                q += 1
                idx, q = _uleb(data, q)
                if kind == 1 and name == "__wbindgen_externrefs":  # table export
                    export_idx = idx
        pos = end
    return tables, export_idx


def main(argv):
    if len(argv) != 2:
        print("usage: check_externref.py <module.wasm>", file=sys.stderr)
        return 2
    tables, export_idx = inspect(argv[1])
    if export_idx is None:
        # No reference-types externref heap — nothing wasm-opt could have broken.
        return 0
    if export_idx >= len(tables):
        print(f"__wbindgen_externrefs points at table {export_idx}, "
              f"but the module has only {len(tables)} tables", file=sys.stderr)
        return 1
    elem_type, has_max = tables[export_idx]
    if elem_type != EXTERNREF:
        kind = "funcref" if elem_type == FUNCREF else hex(elem_type)
        print(f"__wbindgen_externrefs is bound to a {kind} table, not externref "
              f"— wasm-opt reordered the tables (binaryen too old)", file=sys.stderr)
        return 1
    if has_max:
        print("__wbindgen_externrefs table has a fixed maximum and can't grow "
              "— wasm-opt capped it (binaryen too old)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
