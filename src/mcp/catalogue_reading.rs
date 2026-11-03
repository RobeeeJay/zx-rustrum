//! The tools for saying what a program's data is and watching what it does to
//! itself. Apart from the main list only because that had grown past a file.

use crate::mcp::catalogue::{address, prop, schema_tool, tool};
use crate::mcp::json::Json;

pub fn tools() -> Vec<Json> {
    vec![
        schema_tool(
            "export_flowchart",
            "Write one routine as a flow chart in SVG: top to bottom in address order, from \
             the code that ran, with a box for each run of instructions showing what it calls \
             and what the notes say, a diamond for each decision with how often each way was \
             taken, jumps forward down the right and loops back up the left, and jumps out to \
             other routines named at the side. This is the picture of a game loop the call \
             graph cannot give, since a main loop is jumped round, not called.",
            [
                ("address", address("the routine's first instruction")),
                ("path", prop("string", "where to write the .svg")),
                (
                    "max_blocks",
                    prop("integer", "most blocks to draw, 1-400 (default 80)"),
                ),
            ],
            &["address", "path"],
        ),
        schema_tool(
            "export_call_graph",
            "Write who called whom as an SVG to keep beside a disassembly: the busiest \
             routines laid out in layers left to right, named from the notes, a thicker line \
             for a call made more often, a dashed one going back for a loop, and each \
             routine's description as its tooltip. Needs watch_routines on while the program \
             ran.",
            [
                ("path", prop("string", "where to write the .svg")),
                (
                    "limit",
                    prop("integer", "how many routines, 2-200 (default 60)"),
                ),
                (
                    "from",
                    address("only routines from here (default $0000), such as the program's own"),
                ),
                ("to", address("only routines up to here (default $FFFF)")),
            ],
            &["path"],
        ),
        schema_tool(
            "export_sprites",
            "Write every bitmap described with set_data as a PNG of its own into a directory, \
             named by its label: ink black, paper white, outside a masked sprite's mask \
             transparent, the right way up. Drawn from memory as it is now.",
            [
                ("dir", prop("string", "the directory to write into")),
                (
                    "scale",
                    prop(
                        "integer",
                        "screen pixels to each of the sprite's, 1-16 (default 4)",
                    ),
                ),
            ],
            &["dir"],
        ),
        tool(
            "unexplained",
            "What has run and has no explanation yet, biggest gap first: for each labelled \
             stretch of code, whether it says what it does and how many of its instructions \
             have a comment, and which routines have no name. It only counts: a bad comment \
             counts the same as a good one. Use it to choose what to read next.",
            [
                ("from", address("lowest address (default $5B00)")),
                ("to", address("highest address (default $FFFF)")),
                ("limit", prop("integer", "how many to list (default 30)")),
            ],
        ),
        tool(
            "self_modifying",
            "What has rewritten code that had already run, and with what: a patched \
             operand, a jump turned on and off, or a loader moving a block over its own \
             start. Only writes that changed the byte count. Needs watch_routines on while \
             it happened.",
            [
                ("from", address("lowest address (default $5B00)")),
                ("to", address("highest address (default $FFFF)")),
                (
                    "limit",
                    prop("integer", "how many runs to list (default 40)"),
                ),
            ],
        ),
        schema_tool(
            "watch_value",
            "Start logging every change to a byte or a few: when, by which instruction, \
             from what to what. This answers \"what changes the lives, and when\" without \
             stopping the machine. Costs a comparison per write while the observer is on, \
             and turns the observer on. The latest 4096 changes are kept. clear: true stops \
             every watch.",
            [
                ("address", address("the byte, or the first of them")),
                ("length", prop("integer", "how many bytes (default 1)")),
                ("clear", prop("boolean", "stop all the watches")),
            ],
            &[],
        ),
        tool(
            "value_log",
            "What the watches have seen. Says which instructions made the changes and how \
             many each, then the changes themselves, latest last.",
            [
                ("address", address("only this byte (default all of them)")),
                (
                    "limit",
                    prop("integer", "how many changes to list (default 40)"),
                ),
            ],
        ),
        schema_tool(
            "force_branch",
            "Make a conditional go the way it has not gone, on a fork of the machine, and \
             say what code that reached. Runs until the machine reaches the instruction, \
             turns over the flag it tests (or sets B, for DJNZ), runs on, then puts the \
             machine back exactly as it was and keeps only the record of what ran, marked \
             as forced. The flag it turned over is the only thing changed, so what follows \
             may be something the program could never do: read it, don't believe it. Needs \
             watch_routines on.",
            [
                ("address", address("the conditional instruction")),
                (
                    "side",
                    prop(
                        "string",
                        "taken or not_taken (default whichever has not been seen)",
                    ),
                ),
                (
                    "state",
                    prop("string", "a saved state to start the fork from"),
                ),
                (
                    "reach_frames",
                    prop("integer", "how long to wait for it (default 500)"),
                ),
                (
                    "frames",
                    prop("integer", "how long to run after (default 100)"),
                ),
            ],
            &["address"],
        ),
        schema_tool(
            "set_data",
            "Say what a stretch of memory holds, so the listing and the assembly export \
             show it as that: words as DEFW with names for the addresses, text as DEFM, \
             a bitmap as DEFB rows with the picture above them. It is a \
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
                (
                    "bottom_up",
                    prop(
                        "boolean",
                        "the first row in memory is the bottom of the picture, for a program \
                         that draws up the screen. The picture is then turned the right way \
                         up, and the rows stay in memory order.",
                    ),
                ),
                (
                    "masked",
                    prop(
                        "boolean",
                        "each byte of picture has a mask byte before it, as in sprites \
                         drawn over a background: width is still the picture's bytes a row, \
                         and only those are drawn in the picture",
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
             a label of its own, object[6].x if it does not. A count of 0 takes it back.",
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
