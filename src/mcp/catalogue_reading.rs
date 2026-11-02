//! The tools for saying what a program's data is and watching what it does to
//! itself. Apart from the main list only because that had grown past a file.

use crate::mcp::catalogue::{address, prop, schema_tool};
use crate::mcp::json::Json;

pub fn tools() -> Vec<Json> {
    vec![
        schema_tool(
            "set_data",
            "Say what a stretch of memory holds, so the listing and the assembly export \
             show it as that: words as DEFW with names for the addresses, text as DEFM, \
             a bitmap as DEFB rows with the picture beside each in # and dots. It is a \
             claim you are making: nothing checks it, and the bytes come out the same \
             whatever you say. kind \"bytes\" takes a claim back.",
            [
                ("address", address("where it starts")),
                (
                    "kind",
                    prop("string", "words, text, bitmap, or bytes to take it back"),
                ),
                (
                    "length",
                    prop("integer", "how many words, or how many bytes of text"),
                ),
                ("width", prop("integer", "a bitmap's bytes per row")),
                ("rows", prop("integer", "a bitmap's rows")),
                (
                    "header",
                    prop(
                        "integer",
                        "bytes before a bitmap that are not picture, such as its size (default 0)",
                    ),
                ),
            ],
            &["address", "kind"],
        ),
        schema_tool(
            "define_struct",
            "Name the fields of a record, for struct_array and bind_struct to use. \
             Defining a name again replaces it.",
            [
                ("name", prop("string", "what it is called")),
                ("size", prop("integer", "bytes in one")),
                (
                    "fields",
                    prop("string", "offsets and names, such as \"0=type 1=x 2=y\""),
                ),
            ],
            &["name", "size", "fields"],
        ),
        schema_tool(
            "struct_array",
            "Say that records of one kind lie side by side here, so an absolute address \
             into them is named in the listing: $5D31 reads as rocket.x if the record has \
             a label of its own, objects[6].x if it does not. A count of 0 takes it back.",
            [
                ("address", address("where the first one starts")),
                ("struct", prop("string", "a name given to define_struct")),
                ("count", prop("integer", "how many")),
            ],
            &["address", "struct", "count"],
        ),
        schema_tool(
            "bind_struct",
            "Say that over this range of code a register points at a record of this \
             kind, so (IX+$05) is shown as (IX+speed). The narrowest range covering an \
             instruction wins, so one handler can say what its own fields mean under a \
             wider binding for the whole table. Binding the same register and range to \
             \"\" takes it back.",
            [
                ("register", prop("string", "IX or IY")),
                ("struct", prop("string", "a name given to define_struct")),
                ("from", address("first address of the code")),
                ("to", address("last address of the code")),
            ],
            &["register", "struct", "from", "to"],
        ),
    ]
}
