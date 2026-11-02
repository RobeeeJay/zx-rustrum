//! What the server says it can do, in the shape MCP asks for it.
//!
//! Each tool carries a description written for the thing reading it: what it
//! is for, what it costs, and what has to have happened first. A model that
//! calls `routines` before `watch_routines` gets nothing, so the descriptions
//! say so rather than leaving it to be discovered.

use crate::mcp::json::Json;

/// A JSON Schema property.
pub(super) fn prop(kind: &str, description: &str) -> Json {
    Json::obj([
        ("type", Json::str(kind)),
        ("description", Json::str(description)),
    ])
}

/// An address property: a number, or a string like "$8000".
pub(super) fn address(description: &str) -> Json {
    Json::obj([
        (
            "type",
            Json::arr(vec![Json::str("integer"), Json::str("string")]),
        ),
        ("description", Json::str(description)),
    ])
}

pub(super) fn tool<const N: usize>(
    name: &str,
    description: &str,
    properties: [(&str, Json); N],
) -> Json {
    schema_tool(name, description, properties, &[])
}

pub(super) fn schema_tool<const N: usize>(
    name: &str,
    description: &str,
    properties: [(&str, Json); N],
    required: &[&str],
) -> Json {
    Json::obj([
        ("name", Json::str(name)),
        ("description", Json::str(description)),
        (
            "inputSchema",
            Json::obj([
                ("type", Json::str("object")),
                (
                    "properties",
                    Json::Obj(
                        properties
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect(),
                    ),
                ),
                (
                    "required",
                    Json::arr(required.iter().map(|r| Json::str(*r)).collect()),
                ),
            ]),
        ),
    ])
}

pub fn tools() -> Json {
    let mut all = own_tools();
    all.extend(crate::mcp::catalogue_reading::tools());
    Json::arr(all)
}

fn own_tools() -> Vec<Json> {
    vec![
        tool(
            "machine_info",
            "What machine is running, where it has got to, what is loaded, and whether \
             the observer is on. A good first call.",
            [],
        ),
        schema_tool(
            "set_machine",
            "Switch model and reset. The ROM must be findable: roms/ beside the emulator, \
             or the preferences directory.",
            [(
                "model",
                prop(
                    "string",
                    "48k, 128k, +2a, +3, zx81 or zx81-1k. A ZX81 is a different machine, not \
                     a Spectrum with less in it: the CPU draws its screen, so the tools that \
                     watch a ULA have nothing to report there.",
                ),
            )],
            &["model"],
        ),
        tool("reset", "Reset the machine, as the power switch would.", []),
        schema_tool(
            "load_tape",
            "Put a .tap, .tzx or .zip in the deck. By default it also types LOAD \"\" and \
             runs the tape to its end with the ROM loader handed over, which takes a \
             second or two of real time rather than the four minutes the tape lasts. \
             Pass autoload: false to load it without starting it.",
            [
                ("path", prop("string", "the tape file")),
                (
                    "autoload",
                    prop("boolean", "type LOAD \"\" and run it (default true)"),
                ),
            ],
            &["path"],
        ),
        schema_tool(
            "load_program",
            "Load a ZX81 .p or .81 file: that machine's own snapshot, which is its memory \
             from $4009 up. Starts a ZX81 if one is not running.",
            [("path", prop("string", "the .p or .81 file"))],
            &["path"],
        ),
        schema_tool(
            "load_snapshot",
            "Load a .szx, .sna or .z80. The model comes from the file. A .szx is the one \
             that carries the paging, the AY and which joystick is plugged in, and it says \
             what it stepped over rather than losing it quietly. This is the quickest way \
             into the middle of a game.",
            [("path", prop("string", "the snapshot file"))],
            &["path"],
        ),
        schema_tool(
            "load_recording",
            "Load an .rzx recording: a snapshot plus every byte the machine read from a \
             port afterwards. play_recording then replays somebody else's session, which \
             is how to watch a game being played without playing it.",
            [("path", prop("string", "the recording"))],
            &["path"],
        ),
        tool(
            "play_recording",
            "Play a loaded recording forward. A frame of a recording is a number of opcode \
             fetches rather than a number of T-states, and the interrupt comes at the \
             recording's own frame boundary.",
            [("frames", prop("integer", "how many frames (default 1)"))],
        ),
        tool(
            "recording_info",
            "How long the loaded recording is, who made it, how far through it is, and \
             whether it has come adrift — a frame that asks for more input than was \
             recorded is a recording that no longer matches the machine.",
            [],
        ),
        tool(
            "seek_recording",
            "Go to a frame of the recording. Forwards plays on; backwards starts again from \
             the snapshot it carries and plays forward, because nothing can be un-executed.",
            [("frame", prop("integer", "which frame to stop at"))],
        ),
        tool(
            "tape_blocks",
            "What is on the tape: every block, with the header of a standard one decoded — \
             what it loads and at what address. This is the memory map before there is one. \
             A turbo block is the game's own loader at work.",
            [(
                "limit",
                prop("integer", "how many blocks to list (default 60)"),
            )],
        ),
        schema_tool(
            "mount_disk",
            "Put a .dsk or .ipf — or a .zip with one inside — in the +3's drive. An IPF is \
             what the head would have read, protections and all, and cannot be written back \
             to; a DSK can. Read-only unless told otherwise: a game writes \
             its high scores to the disk it loaded from, and doing that to somebody's file \
             uninvited is not on. writable sends writes to the file itself; copy_to writes \
             the image to a new file first and sends them there, leaving the original as it \
             was.",
            [
                (
                    "path",
                    prop("string", "the disk image, or a zip holding one"),
                ),
                (
                    "writable",
                    prop("boolean", "let writes go to this file (default false)"),
                ),
                (
                    "copy_to",
                    prop("string", "copy it here first, and write to the copy"),
                ),
            ],
            &["path"],
        ),
        tool(
            "new_disk",
            "A blank disk, formatted as the machine's own FORMAT formats one — forty tracks \
             of nine 512-byte sectors numbered from $C1, filled with $E5, which is what an \
             empty CP/M directory entry starts with. Writable. Without a path it exists \
             only in the drive.",
            [("path", prop("string", "where to keep it"))],
        ),
        tool(
            "eject_disk",
            "Take the disk out, writing it back first if it has been changed and has \
             somewhere to go.",
            [],
        ),
        tool(
            "disk_info",
            "What is in the drive: its size and format, whether it can be written to and \
             whether it has been, where the head is, whether the motor is turning, and \
             which speed the drive is running at.",
            [],
        ),
        schema_tool(
            "disk_speed",
            "How the drive behaves about time. \"normal\" makes the machine wait as a real \
             drive does — a second for the motor, a step a track for the head, a sector \
             coming round every twenty-second of a second — so a load takes the time it \
             took. \"fastload\" answers at once.",
            [("speed", prop("string", "normal or fastload"))],
            &["speed"],
        ),
        tool(
            "disk_catalogue",
            "What is on the disk, as CAT prints it: the CP/M directory at the front, with \
             each file's size and whether it is read-only or hidden. The hidden ones do not \
             appear in the machine's own CAT and are usually where a game keeps its code.",
            [],
        ),
        schema_tool(
            "read_sector",
            "A sector off the disk, by track and sector number — the number in its address \
             mark, which is what the controller matches on, not where it sits on the track. \
             This reads the image rather than driving the drive, so it works whatever the \
             machine is doing.",
            [
                ("track", prop("integer", "which track")),
                ("side", prop("integer", "which side (default 0)")),
                (
                    "sector",
                    address("the sector's number, $C1 and up on a +3 disk"),
                ),
                (
                    "offset",
                    prop("integer", "where in the sector to start (default 0)"),
                ),
                (
                    "length",
                    prop("integer", "how many bytes (default the whole sector)"),
                ),
            ],
            &["track"],
        ),
        tool(
            "disk_activity",
            "Which sectors the machine has read and written lately, brightest first. The \
             map fades over a couple of seconds, so this is what a game is loading now \
             rather than everything it has ever loaded.",
            [],
        ),
        tool(
            "loader",
            "Which loader the machine is sitting in, read off the sampling loop it is \
             counting pulses in — Speedlock, Bleepload, Alkatraz and the rest. Nine are \
             recognised, taken off the tapes rather than from a list.",
            [],
        ),
        tool(
            "step",
            "Step instructions, printing each one as it goes. With over: true a CALL or a \
             block instruction is run to completion rather than stepped into.",
            [
                ("count", prop("integer", "how many (default 1)")),
                ("over", prop("boolean", "step over calls (default false)")),
            ],
        ),
        tool(
            "run_frames",
            "Run whole TV frames — 69,888 T-states each on a 48K, 70,908 on a 128K. Stops \
             early on a breakpoint or a watched event.",
            [("frames", prop("integer", "how many (default 1)"))],
        ),
        tool(
            "run_tstates",
            "Run a number of T-states. Use this to stop part-way through a frame: the ULA \
             draws the screen between T-state 14,336 and 57,344 on a 48K, so where in the \
             frame a routine runs decides whether its writes are seen this frame or next.",
            [("tstates", prop("integer", "how many"))],
        ),
        tool(
            "run_until",
            "Run until the program reaches an address. This is the breakpoint: give one \
             address or several, and it stops at whichever comes first.",
            [
                ("address", address("where to stop")),
                (
                    "addresses",
                    Json::obj([
                        ("type", Json::str("array")),
                        ("items", address("an address")),
                        ("description", Json::str("several places to stop")),
                    ]),
                ),
                (
                    "max_frames",
                    prop("integer", "give up after this many frames (default 1000)"),
                ),
            ],
        ),
        tool(
            "watch_events",
            "Stop on what a program does rather than on where it is: a write to the display \
             file, the beeper or the sound chip being touched, the frame interrupt, a call \
             into the ROM, any IN, any OUT. Switch on what you want and pass run: true. \
             This is how to find the drawing routine, the keyboard read or the music player \
             without knowing an address.",
            [
                (
                    "screen",
                    prop("boolean", "a write anywhere in the display file"),
                ),
                (
                    "beeper",
                    prop("boolean", "the speaker or MIC bit of port $FE"),
                ),
                ("ay", prop("boolean", "any access to the sound chip")),
                (
                    "interrupt",
                    prop("boolean", "the frame interrupt being taken"),
                ),
                (
                    "rom",
                    prop("boolean", "code entering the ROM from outside it"),
                ),
                ("port_in", prop("boolean", "any IN")),
                ("port_out", prop("boolean", "any OUT")),
                (
                    "write_to",
                    address("stop when anything writes to this address; null to stop watching"),
                ),
                (
                    "write_to_end",
                    address("the far end of a range of addresses to watch"),
                ),
                ("run", prop("boolean", "run until one of them happens")),
                (
                    "max_frames",
                    prop("integer", "how long to wait (default 1000)"),
                ),
            ],
        ),
        schema_tool(
            "press_keys",
            "Press keys, hold them, and let go. Several together is a chord: [\"CAPS SHIFT\", \
             \"1\"]. The ROM scans the keyboard once a frame and wants a key on two scans \
             running, so a key is held for ten frames by default. This is how a game is \
             driven to the part worth looking at, and how to find the input routine: press \
             something and see who reads port $FE.",
            [
                (
                    "keys",
                    Json::obj([
                        (
                            "type",
                            Json::arr(vec![Json::str("array"), Json::str("string")]),
                        ),
                        (
                            "items",
                            prop(
                                "string",
                                "a key: A-Z, 0-9, ENTER, SPACE, CAPS SHIFT, SYMBOL SHIFT",
                            ),
                        ),
                        ("description", Json::str("the keys to hold down together")),
                    ]),
                ),
                (
                    "frames",
                    prop("integer", "how long to hold them (default 10)"),
                ),
                (
                    "then_frames",
                    prop("integer", "frames to run afterwards (default 10)"),
                ),
            ],
            &["keys"],
        ),
        schema_tool(
            "type_text",
            "Type a BASIC line the way somebody at the keyboard would, keywords and all: \
             LOAD *\"m\";1;\"prog\", FORMAT \"m\";1;\"cart\", CAT 1, SAVE \"game\"CODE. A \
             Spectrum's keywords are one key each rather than spelled out, and the ones in \
             extended mode — CAT, FORMAT, VERIFY, MERGE, CIRCLE, INVERSE — cannot be typed \
             any other way. Symbols behind SYMBOL SHIFT are found too. Anything inside \
             quotes is typed letter by letter, so a file called \"info\" is not IN and \"fo\". \
             Letters come out in the machine's own case. A line starts with a keyword: at \
             the start of a line a letter key types a whole word, and asking for a letter \
             there is an error saying so.",
            [
                ("text", prop("string", "what to type")),
                (
                    "enter",
                    prop("boolean", "press ENTER at the end (default true)"),
                ),
                (
                    "frames",
                    prop("integer", "frames a key is held (default 6)"),
                ),
            ],
            &["text"],
        ),
        schema_tool(
            "printout",
            "What the ZX Printer or Alphacom 32 has printed: the paper as a PNG, a pixel a \
             dot, and any text on it read back through the machine's font. COPY prints the \
             top 22 rows of the screen, LPRINT and LLIST print text. Needs a printer fitted \
             (fit with what: zx_printer or alphacom32).",
            [("image", prop("boolean", "include the PNG (default true)"))],
            &[],
        ),
        schema_tool(
            "mouse",
            "Move whichever mouse is fitted and set its buttons, then run some frames so the \
             program sees it. dx right and dy down are in the machine's pixels. A Kempston \
             mouse is two eight-bit counters that wrap, so a program moves its pointer by \
             the difference between reads; an AMX mouse queues the steps and its PIO \
             interrupts once for each, but only once the program has turned its interrupts \
             on. Needs a mouse fitted (fit with what: kempston_mouse or amx_mouse).",
            [
                ("dx", prop("integer", "pixels to the right; negative is left (default 0)")),
                ("dy", prop("integer", "pixels down; negative is up (default 0)")),
                ("left", prop("boolean", "left button held (default false)")),
                ("middle", prop("boolean", "middle button held, AMX only (default false)")),
                ("right", prop("boolean", "right button held (default false)")),
                (
                    "frames",
                    prop("integer", "frames to run with it so (default 5)"),
                ),
            ],
            &[],
        ),
        tool(
            "step_forward",
            "Step, keeping what it would take to undo it. Twenty of these can be taken back \
             with step_back — which is what you want a moment after an instruction has done \
             something you did not expect.",
            [("count", prop("integer", "how many (default 1)"))],
        ),
        tool(
            "step_back",
            "Undo instructions stepped with step_forward, newest first. Running does not \
             keep what it would take: the writes of a whole frame cost more than the frame.",
            [("count", prop("integer", "how many to undo (default 1)"))],
        ),
        tool(
            "paging",
            "Which ROM and RAM bank a 128K has paged in, and switching them. A disassembly \
             that does not say which bank was at $C000 is a disassembly of somewhere else.",
            [(
                "page_register",
                prop("integer", "a byte to write to port $7FFD"),
            )],
        ),
        tool(
            "blocks",
            "The runs a listing is divided into — code here, data there — kept in the notes \
             beside the labels. With work_out it takes them from what has run, keeping \
             anything already written down.",
            [
                ("work_out", prop("boolean", "take them from what has run")),
                ("from", address("the start of a block to mark by hand")),
                ("to", address("its last byte")),
                ("kind", prop("string", "code or data")),
            ],
        ),
        tool(
            "profile",
            "Where the time goes, which is not the same question as what runs most often: a \
             routine called twice a frame that takes half of it matters more than one called \
             two hundred times that does not. Runs the machine for a while with the profiler \
             on and reports the ranking.",
            [
                ("frames", prop("integer", "how long to watch (default 50)")),
                (
                    "limit",
                    prop("integer", "how many routines to list (default 20)"),
                ),
            ],
        ),
        tool(
            "registers",
            "Every register, the flags spelled out, and where the machine is in the frame.",
            [],
        ),
        schema_tool(
            "read_memory",
            "Read memory as hex, with the printable bytes beside it. Up to 4096 bytes a \
             call. Without a bank this is what the CPU sees through its current paging; \
             with one it is that RAM bank whatever is paged in.",
            [
                ("address", address("where to start")),
                ("length", prop("integer", "how many bytes (default 256)")),
                ("bank", prop("integer", "a 128K RAM bank, 0-7")),
            ],
            &["address"],
        ),
        schema_tool(
            "write_memory",
            "Poke bytes in. For testing a theory — freeze the lives counter, blank a sprite \
             and see what stops moving — not for patching a file: nothing is written to disc.",
            [
                ("address", address("where to write")),
                (
                    "bytes",
                    prop("string", "hex bytes, \"3E 00 32\", or a list of numbers"),
                ),
            ],
            &["address", "bytes"],
        ),
        tool(
            "memory_activity",
            "Where the program keeps things: reads, writes and whether anything was executed, \
             counted per 256-byte page, with a guess at what each page is for. Also whether a \
             back buffer has been detected — a screen built somewhere other than the display \
             file and blitted across. This is the map to read before disassembling anything.",
            [
                ("from", address("lowest address (default $4000)")),
                ("to", address("highest address (default $FFFF)")),
                (
                    "include_quiet",
                    prop("boolean", "list pages nothing has touched"),
                ),
                ("limit", prop("integer", "how many pages (default 40)")),
            ],
        ),
        tool(
            "find_bytes",
            "Where a sequence of bytes appears in memory: a sprite you have the bytes of, a \
             string the game prints, a value you are hunting for.",
            [
                (
                    "bytes",
                    prop("string", "hex bytes, \"3E 00 32\", or a list of numbers"),
                ),
                ("text", prop("string", "ASCII to look for instead")),
                ("value", prop("integer", "a single byte to look for")),
                ("from", address("where to start (default $4000)")),
                ("to", address("where to stop (default $FFFF)")),
                (
                    "limit",
                    prop("integer", "how many hits to list (default 40)"),
                ),
            ],
        ),
        tool(
            "changed_since",
            "What is different in memory from a state saved earlier. This is how a variable \
             is found: save the machine, lose a life, ask what changed, and the counter is \
             among the handful of addresses that come back. Narrow it with value. The \
             display file is left out unless asked for, since it changes every frame and \
             says nothing.",
            [
                (
                    "name",
                    prop(
                        "string",
                        "the saved state to compare with (default \"last\")",
                    ),
                ),
                (
                    "value",
                    prop("integer", "only addresses that now hold this byte"),
                ),
                ("from", address("lowest address (default $4000)")),
                ("to", address("highest address (default $FFFF)")),
                (
                    "include_screen",
                    prop("boolean", "include $4000-$5AFF (default false)"),
                ),
                ("limit", prop("integer", "how many to list (default 60)")),
            ],
        ),
        tool(
            "save_state",
            "Take a snapshot of the whole machine and keep it under a name, so an experiment \
             can be undone. With a path it is also written as a .sna file.",
            [
                ("name", prop("string", "what to call it (default \"last\")")),
                ("path", prop("string", "also write it here as .sna")),
            ],
        ),
        tool(
            "restore_state",
            "Put a saved machine back, by name or from a .sna or .z80 file.",
            [
                ("name", prop("string", "a name given to save_state")),
                ("path", prop("string", "a snapshot file instead")),
            ],
        ),
        tool(
            "screen",
            "What is on the screen: a PNG you can look at, a sketch of the 32x24 character \
             cells for when you cannot, and what the attributes are set to. This is how to \
             check whether a change did what you thought — did the sprite disappear, is the \
             score where you think it is.",
            [
                (
                    "image",
                    prop("boolean", "send the picture as well (default true)"),
                ),
                (
                    "border",
                    prop("boolean", "include the border (default false)"),
                ),
                (
                    "path",
                    prop("string", "write the PNG here instead of sending it"),
                ),
            ],
        ),
        schema_tool(
            "graphics",
            "Read memory as characters and sprites — eight bytes a character, one bit a \
             pixel — and draw them. Point it at a candidate address and see whether letters, \
             a sprite or rubbish comes out. This is how graphics are found.",
            [
                ("address", address("where the graphics might start")),
                ("count", prop("integer", "how many characters (default 16)")),
                (
                    "across",
                    prop("integer", "how many to a row, 1-32 (default 8)"),
                ),
                (
                    "image",
                    prop("boolean", "send a picture as well as the text"),
                ),
            ],
            &["address"],
        ),
        tool(
            "disassemble",
            "Disassemble, with your labels and comments against the lines that have them, \
             and a note of whether each address has been executed or only read while \
             watching — which is how code is told from data.",
            [
                ("address", address("where to start (default PC)")),
                (
                    "count",
                    prop("integer", "how many instructions (default 32)"),
                ),
                (
                    "comments",
                    prop("boolean", "include labels and comments (default true)"),
                ),
            ],
        ),
        tool(
            "watch_routines",
            "Switch the observer on. It attributes every write, read, port access and loop \
             to the routine making it, which is what routines, routine, call_graph, code_map \
             and autodoc read. Run the machine afterwards — a few hundred frames of a game \
             is plenty. It costs a branch on every access, so switch it off when finished.",
            [
                ("enabled", prop("boolean", "on or off (default on)")),
                ("clear", prop("boolean", "forget what was seen so far")),
            ],
        ),
        tool(
            "routines",
            "Every routine seen called, with how often, how much work it did, what it wrote \
             to the screen and the attributes, what it read, and how big it is. Sorted by \
             work done unless told otherwise.",
            [
                ("sort", prop("string", "work, calls, writes or address")),
                ("limit", prop("integer", "how many to list (default 50)")),
            ],
        ),
        schema_tool(
            "routine",
            "One routine in full: its extent, its writes and reads per call, its ports, its \
             loops and how many times round they went, where it exits, and what was in the \
             registers when it was entered.",
            [("address", address("the routine's entry point"))],
            &["address"],
        ),
        schema_tool(
            "mount_cartridge",
            "Put an .mdr cartridge — or a .zip with one inside — in a microdrive. Needs an \
             Interface 1: fit {\"what\": \"if1\"} first. Drives are numbered from 1, as the \
             machine numbers them. Read-only unless told otherwise, because a program writes \
             to the cartridge it loaded from: writable sends writes to the file, copy_to \
             writes a copy first and sends them there.",
            [
                ("path", prop("string", "the .mdr, or a zip holding one")),
                ("drive", prop("integer", "which drive, from 1 (default 1)")),
                (
                    "writable",
                    prop("boolean", "let writes go to this file (default false)"),
                ),
                (
                    "copy_to",
                    prop("string", "copy it here first, and write to the copy"),
                ),
            ],
            &["path"],
        ),
        schema_tool(
            "new_cartridge",
            "A blank formatted cartridge in a drive, writable. The machine's own FORMAT \
             writes a sector of test pattern and leaves it there, so a cartridge formatted \
             by the ROM reports a kilobyte less free than one made here.",
            [
                ("drive", prop("integer", "which drive, from 1 (default 1)")),
                ("name", prop("string", "the cartridge's name (default \"blank\")")),
                ("sectors", prop("integer", "how many, up to 254 (default 180)")),
                ("path", prop("string", "write it here, so it survives the session")),
            ],
            &[],
        ),
        schema_tool(
            "eject_cartridge",
            "Take the cartridge out, writing it back to its file first if the machine \
             changed it and the mount allowed it.",
            [("drive", prop("integer", "which drive, from 1 (default 1)"))],
            &[],
        ),
        tool(
            "microdrive_info",
            "The chain: how many drives, what is in each, which one is turning and where its \
             head is, and whether the Interface 1's ROM is there — without that ROM the \
             machine has no microdrive commands at all.",
            [],
        ),
        schema_tool(
            "cartridge_catalogue",
            "What is on a cartridge: its name, its files with their sizes, how much is free, \
             and which sectors do not add up. A worn cartridge really does have bad sectors, \
             so that is a fact about the tape rather than a fault in the reading.",
            [("drive", prop("integer", "which drive, from 1 (default 1)"))],
            &[],
        ),
        tool(
            "hardware",
            "What can be plugged into the back of the machine, what is plugged in now, and \
             how far each thing is emulated. The Interface 1 and its microdrives, the three \
             Multifaces, the Currah µSpeech, the Fuller Audio Box and the SpecDrum do \
             something; each of the ones with a ROM says whether that ROM was found. Start \
             here before fit.",
            [],
        ),
        schema_tool(
            "fit",
            "Plug something in, or take it off. `what` is the peripheral's key — if1, \
             uspeech, fuller, specdrum, multiface1, multiface128, multiface3, music_machine \
             — and a wrong one is answered with the list. Fitting an Interface 1 builds its microdrives; fitting \
             a µSpeech loads both its ROMs. A box whose ROM is missing is fitted and says so, \
             because a machine that ignores a peripheral looks the same as a broken one.",
            [
                ("what", prop("string", "the peripheral's key")),
                (
                    "fitted",
                    prop("boolean", "on or off (default true, meaning fit it)"),
                ),
                (
                    "microdrives",
                    prop("integer", "how many are on the Interface 1's chain, 1 to 8"),
                ),
            ],
            &["what"],
        ),
        tool(
            "red_button",
            "Press the Multiface's red button. The machine takes an NMI wherever it is — \
             interrupts disabled or not — and its ROM pages in over the bottom 16K and draws \
             a menu, with every register of the stopped program still in it. run_frames, then \
             screen, then press_keys to work the menu. save_state is the quieter way to keep \
             a machine; this is the way a person did it in 1986, and the way to see what a \
             game's own code looked like at that moment.",
            [],
        ),
        tool(
            "sound_state",
            "What the machine is making a noise with: the beeper bit of port $FE, the AY's \
             fourteen registers with what they mean channel by channel — tone, noise, \
             volume, period in Hz — and whatever is plugged into the back. A Fuller Box's \
             chip is read here too, which is the only sound chip a 48K can have. The \
             SpecDrum's converter says what it is playing this instant, and a µSpeech says \
             what it has been saying. Watch the ay or beeper events to find the routine \
             doing it.",
            [],
        ),
        schema_tool(
            "set_sound",
            "Silence part of the sound, or bring it back: beeper, ay (both the machine's \
             chip and a Fuller Box's), hardware (the SpecDrum and the µSpeech). A muted part \
             is silenced in the mixer and the machine is not told, so a program goes on \
             playing it. Useful for hearing one thing at a time — a game's music without \
             its beeper effects.",
            [
                ("beeper", prop("boolean", "the machine's own speaker")),
                ("ay", prop("boolean", "the sound chips")),
                ("hardware", prop("boolean", "what the add-ons make")),
            ],
            &[],
        ),
        tool(
            "frame_timing",
            "Where the beam is, and when in the frame a watched routine ran. On this machine \
             when is half the question: the ULA puts the picture out as it goes, so a write \
             above the beam is seen this frame and one below it waits for the next — which \
             is what a flickering sprite is.",
            [(
                "address",
                address("a routine's entry point, to ask about that one"),
            )],
        ),
        tool(
            "call_graph",
            "Who called whom, and how often. A line back to something already reached is a \
             loop in the program.",
            [("limit", prop("integer", "how many edges (default 60)"))],
        ),
        tool(
            "code_map",
            "Which addresses were executed and which were only read: what is code and what \
             is data, as far as the machine has got. Blocks in $4000-$5AFF are the screen \
             itself.",
            [(
                "min_length",
                prop("integer", "shortest data block to report (default 8)"),
            )],
        ),
        tool(
            "coverage",
            "How far watching has got, and where to go next: what has run, what the code \
             that has run could reach but has not, what has only been read, and which \
             conditionals have only ever gone one way. Unreached stretches are the code a \
             disassembly built from watching is missing, and a one-sided branch says what \
             to make happen to reach it.",
            [
                ("from", address("lowest address (default $5B00, above the screen)")),
                ("to", address("highest address (default $FFFF)")),
                (
                    "limit",
                    prop("integer", "most stretches and branches to list (default 60)"),
                ),
            ],
        ),
        tool(
            "indirect_jumps",
            "Every JP (HL), JP (IX) and JP (IY) that has run: where each went, how often, \
             and the instruction before it, which is the one to read when the jump is a \
             one-byte trampoline. Then those targets found side by side in memory, which \
             is a jump table, with the entries nothing has jumped to yet. The code names \
             none of this, so it is how dispatch is found.",
            [
                ("from", address("lowest address to look for tables (default $5B00)")),
                ("to", address("highest address to look for tables (default $FFFF)")),
            ],
        ),
        tool(
            "load_symbols",
            "Read names for addresses out of a symbol file — a ROM disassembly turned into \
             one by tools/rom-symbols.py, or a game's by tools/skool-symbols.py — and build \
             the table of routines recognisable by their first bytes. With no path it reads \
             the files for this machine from the preferences directory. Names loaded this \
             way appear in disassemble, and a CALL to a named address says which.",
            [(
                "path",
                prop("string", "a symbol file; the machine's own by default"),
            )],
        ),
        tool(
            "symbols",
            "The names that came from a symbol file, in an address range.",
            [
                ("from", address("lowest address")),
                ("to", address("highest address")),
            ],
        ),
        schema_tool(
            "identify",
            "What the code at an address is, if its first twelve bytes are a routine that is \
             known. Games copy ROM routines into RAM and the bytes hash the same wherever \
             they land, so a copy is named after the original and said to be one.",
            [("address", address("where the code starts"))],
            &["address"],
        ),
        schema_tool(
            "xrefs",
            "Everything that refers to an address, both ways round: which routines were \
             watched calling it, which hammered it, which read its page, and which \
             instruction in memory names it. The watched answers are facts about the run; \
             the instructions are a search, and some of what it finds will be data that \
             happens to look like code. This is the question to ask when naming a variable.",
            [
                ("address", address("the address to look for")),
                (
                    "search_from",
                    address("where the search starts (default $4000)"),
                ),
                ("search_to", address("where it ends (default $FFFF)")),
            ],
            &["address"],
        ),
        tool(
            "autodoc",
            "Guesses at what routines do, from what they touch — ports, screen ranges, ROM \
             calls, block moves — and from what they were measured doing. It hedges where \
             the evidence is thin and leaves code with no tell unnamed. Guesses, not answers.",
            [(
                "entries",
                Json::obj([
                    ("type", Json::str("array")),
                    ("items", address("a routine entry point")),
                    (
                        "description",
                        Json::str("which routines to look at; the busiest watched ones by default"),
                    ),
                ]),
            )],
        ),
        schema_tool(
            "set_comment",
            "Write a label, a comment, or both against an address. What is typed here is \
             yours: a later guess never overwrites it.",
            [
                ("address", address("the address")),
                ("label", prop("string", "a name for it")),
                ("comment", prop("string", "what it does")),
            ],
            &["address"],
        ),
        tool(
            "comments",
            "Read back the labels and comments, with the guesses marked as guesses.",
            [
                ("from", address("lowest address (default $0000)")),
                ("to", address("highest address (default $FFFF)")),
            ],
        ),
        schema_tool(
            "export_asm",
            "Write the program in RAM ($4000-$FFFF) as assembly source that assembles back \
             into the same bytes: instructions where code has been seen to run, DEFB \
             elsewhere, with every label and comment from the notes. The ROMs it was built \
             against are named in its header by CRC32 and left out.",
            [("path", prop("string", "where to write the .asm file"))],
            &["path"],
        ),
        schema_tool(
            "export_listing",
            "Write the whole annotated disassembly to a file: every label and comment \
             against the code, and the parts that were never executed left as bytes rather \
             than turned into instructions nobody ran. This is the thing being built.",
            [
                ("path", prop("string", "where to write it")),
                ("from", address("lowest address (default $4000)")),
                ("to", address("highest address (default $FFFF)")),
                (
                    "facts",
                    prop(
                        "boolean",
                        "the measured lines and marks: who uses what, what a routine was seen \
                         doing, branches that only went one way, code rewritten or forced \
                         (default true)",
                    ),
                ),
            ],
            &["path"],
        ),
        tool(
            "save_comments",
            "Write the notes to their file beside the tape or snapshot — plain text, one \
             line an address, readable and diffable without the emulator.",
            [],
        ),
    ]
}
