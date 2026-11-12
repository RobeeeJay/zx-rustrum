//! The program in RAM as assembly source, and that source assembled back.
//!
//! The promise is that the file assembles into the bytes it came from, and
//! the only honest test of that is an assembler. None is part of the build,
//! so these use sjasmplus when one is named in ZXRS_Z80ASM or found on the
//! path, and skip that part otherwise — the same bargain as the ROMs.

use std::path::PathBuf;
use zx_rustrum::asmexport;
use zx_rustrum::machine::{Spectrum, FRAME_T};
use zx_rustrum::notes::Notes;

fn assembler() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ZXRS_Z80ASM").map(PathBuf::from) {
        return p.exists().then_some(p);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("sjasmplus"))
            .find(|p| p.exists())
    })
}

/// Assemble with sjasmplus to a raw binary, and hand back the bytes.
fn assemble(asm: &std::path::Path, source: &str, name: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("zxrs-asm-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join("program.asm"), dir.join("program.bin"));
    std::fs::write(&src, source).unwrap();
    let out = std::process::Command::new(asm)
        .arg("--nologo")
        .arg(format!("--raw={}", bin.display()))
        .arg(&src)
        .output()
        .expect("the assembler runs");
    assert!(
        out.status.success(),
        "the assembler refused it:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&bin).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

/// A program full of the awkward cases, run so the tracker has seen its code.
fn awkward() -> (Spectrum, Notes) {
    let mut spec = Spectrum::new();
    let program: &[(u16, &[u8])] = &[
        (0x8000, &[0x3E, 0x05]),             // LD A,$05
        (0x8002, &[0xDD, 0x21, 0x00, 0x90]), // LD IX,$9000
        (0x8006, &[0xDD, 0x7E, 0x03]),       // LD A,(IX+$03)
        (0x8009, &[0xDD, 0x26, 0x07]),       // LD IXH,$07: undocumented
        (0x800C, &[0xED, 0x4C]),             // NEG, a duplicate encoding
        (0x800E, &[0xED, 0x6B, 0x00, 0x90]), // LD HL,($9000), the long form
        (0x8012, &[0xCB, 0x37]),             // SLL A: undocumented
        (0x8014, &[0xCD, 0x00, 0x81]),       // CALL $8100
        (0x8017, &[0xCD, 0x6B, 0x0D]),       // CALL $0D6B, in the ROM
        (0x801A, &[0x18, 0xFE]),             // JR $801A: never runs
        (0x8100, &[0xC9]),                   // RET
        (0x9000, &[1, 2, 3, 4, 5, 6, 7, 8]), // a table
    ];
    for (at, bytes) in program {
        for (i, b) in bytes.iter().enumerate() {
            spec.bus.poke(at + i as u16, *b);
        }
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0x7F00;
    spec.run(FRAME_T);

    let mut notes = Notes::unattached();
    notes.set_label(0x8000, "start");
    notes.set_label(0x8100, "draw sprite");
    notes.set_label(0x8003, "ix_target"); // on an operand byte
    notes.set_label(0x9000, "HL"); // an assembler's own word
    notes.set_label(0x9004, "start"); // taken already
    notes.set_label(0x0D6B, "CLS"); // in the ROM
    notes.set_comment(0x8000, "first line\nsecond line");
    notes.set_comment(0x9000, "the table");
    (spec, notes)
}

/// Its shape, which needs no assembler: an ORG at $4000, labels an assembler
/// will take, the ROM's name as an EQU, the undocumented and the doubly
/// encoded as DEFB with the instruction beside them, and comments kept.
#[test]
fn the_source_is_laid_out_for_an_assembler() {
    let (spec, notes) = awkward();
    let out = asmexport::from_machine(&spec, &notes, None, "awkward");
    let text = &out.text;
    let has = |s: &str| assert!(text.contains(s), "{s:?} in:\n{text}");
    has("        ORG $4000");
    has("draw_sprite:   ; labelled \"draw sprite\"");
    has("CALL draw_sprite");
    has("CLS EQU $0D6B");
    has("CALL CLS");
    has("L_HL:");
    // Prefixed instructions are whole: the byte after the prefix is fetched
    // as an opcode too, and splitting there gave DEFB $DD and an LD HL.
    has("LD IX,L_HL");
    has("LD A,(IX+$03)");
    has("start_9004:");
    has("ix_target EQU $8003");
    has("DEFB $DD,$26,$07");
    has("; LD IXH,$07");
    has("DEFB $ED,$4C");
    has("DEFB $ED,$6B,$00,$90");
    has("DEFB $CB,$37");
    has("; first line");
    has("; second line");
    has("; the table");
    has("CRC32");
    assert!(
        !text.contains("JR $801A"),
        "the JR never ran, so it is data"
    );
    assert!(out.instructions >= 6, "{} instructions", out.instructions);
}

/// Assembled, it is the same 48K it came from, byte for byte.
#[test]
fn the_source_assembles_back_into_the_same_bytes() {
    let Some(asm) = assembler() else {
        eprintln!("no sjasmplus (set ZXRS_Z80ASM); skipping");
        return;
    };
    let (spec, notes) = awkward();
    let out = asmexport::from_machine(&spec, &notes, None, "awkward");
    let bytes = assemble(&asm, &out.text, "awkward");
    let memory: Vec<u8> = (0x4000..=0xFFFFu16).map(|a| spec.bus.peek_raw(a)).collect();
    assert_eq!(bytes.len(), memory.len(), "49,152 bytes from $4000");
    let first = bytes.iter().zip(&memory).position(|(a, b)| a != b);
    assert_eq!(
        first,
        None,
        "first difference at ${:04X}",
        first.unwrap_or(0) + 0x4000
    );
}

/// tools/z80asm.py, the assembler that learns its encodings from the
/// emulator's own disassembler, gives back the same bytes too. It is what the
/// rebuilt games in assemblies/ are made with, where no sjasmplus is
/// installed, so it has to be right about every instruction an export writes.
#[test]
fn the_python_assembler_gives_back_the_same_bytes() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("no python3; skipping");
        return;
    }
    let (spec, notes) = awkward();
    let out = asmexport::from_machine(&spec, &notes, None, "awkward");
    let dir = std::env::temp_dir().join(format!("zxrs-z80asm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join("program.asm"), dir.join("program.bin"));
    std::fs::write(&src, &out.text).unwrap();
    let run = std::process::Command::new("python3")
        .arg("tools/z80asm.py")
        .arg(&src)
        .arg(&bin)
        .arg("--mcp")
        .arg(env!("CARGO_BIN_EXE_mcp"))
        .output()
        .expect("python3 runs");
    assert!(
        run.status.success(),
        "z80asm.py refused it:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let bytes = std::fs::read(&bin).unwrap();

    // DD CB d op decodes as BIT n,(IX+d) whatever the low three bits of op
    // are, so the disassembler alone would have the assembler pick whichever
    // it met first. Only the ones with 6 there are documented, and the first
    // build of Alien 8 came out with 54 bytes wrong this way. The export above
    // has none of these, so they are written out here with what they must be.
    let index = "        ORG $8000\n        BIT 0,(IX+$10)\n        SET 3,(IY-$02)\n        \
                 RES 7,(IX+$7F)\n        RLC (IY+$00)\n        SRL (IX+$01)\n";
    std::fs::write(&src, index).unwrap();
    let run = std::process::Command::new("python3")
        .arg("tools/z80asm.py")
        .arg(&src)
        .arg(&bin)
        .arg("--mcp")
        .arg(env!("CARGO_BIN_EXE_mcp"))
        .output()
        .expect("python3 runs");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stdout)
    );
    let got = std::fs::read(&bin).unwrap();
    let want = [
        0xDD, 0xCB, 0x10, 0x46, 0xFD, 0xCB, 0xFE, 0xDE, 0xDD, 0xCB, 0x7F, 0xBE, 0xFD, 0xCB, 0x00,
        0x06, 0xDD, 0xCB, 0x01, 0x3E,
    ];
    assert_eq!(
        got, want,
        "the documented encodings of the index-register bit instructions"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let memory: Vec<u8> = (0x4000..=0xFFFFu16).map(|a| spec.bus.peek_raw(a)).collect();
    assert_eq!(bytes.len(), memory.len(), "49,152 bytes from $4000");
    let first = bytes.iter().zip(&memory).position(|(a, b)| a != b);
    assert_eq!(
        first,
        None,
        "first difference at ${:04X}: assembled ${:02X}, memory ${:02X}",
        first.unwrap_or(0) + 0x4000,
        first.map(|i| bytes[i]).unwrap_or(0),
        first.map(|i| memory[i]).unwrap_or(0)
    );
}

/// `DEFS $D200-$` is how a rebuilt game keeps a table on its page. When what
/// comes before it has grown past the address, the count is negative, and
/// taking that as nothing moved Alien 8's buffer and stack 168 bytes up
/// without a word. Running past $FFFF was let through the same way.
#[test]
fn the_python_assembler_refuses_space_that_counts_backwards() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("no python3; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("zxrs-z80asm-defs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join("program.asm"), dir.join("program.bin"));
    let refusal = |source: &str| {
        std::fs::write(&src, source).unwrap();
        let run = std::process::Command::new("python3")
            .arg("tools/z80asm.py")
            .arg(&src)
            .arg(&bin)
            .arg("--mcp")
            .arg(env!("CARGO_BIN_EXE_mcp"))
            .output()
            .expect("python3 runs");
        (
            run.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            ),
        )
    };
    let (ok, said) = refusal("        ORG $8000\n        DEFB 1,2,3\n        DEFS $8002-$\n");
    assert!(!ok && said.contains("backwards"), "{said}");
    let (ok, said) =
        refusal("        ORG $8000\n        DEFB 1,2\n        DEFS $8002-$\n        NOP\n");
    assert!(ok, "a DEFS of nothing is fine: {said}");
    let (ok, said) = refusal("        ORG $FFFE\n        DEFB 1,2,3\n");
    assert!(!ok && said.contains("past $FFFF"), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A real program: Border Break, loaded through the ROM and left running so
/// its code has run, assembles back into the same 48K.
#[test]
fn a_real_program_assembles_back_into_the_same_bytes() {
    let (Some(asm), Ok(rom), Ok(tape)) = (
        assembler(),
        std::fs::read("roms/48.rom"),
        zx_rustrum::tape::Tape::load(std::path::Path::new("tapes/borderbreak.tap")),
    ) else {
        eprintln!("need sjasmplus, roms/48.rom and tapes/borderbreak.tap; skipping");
        return;
    };
    let mut spec = Spectrum::new();
    spec.load_rom(&rom);
    spec.reset();
    spec.bus.tape_flash = true;
    for _ in 0..120 {
        spec.run(FRAME_T);
    }
    spec.bus.tape = Some(tape);
    for keys in [
        &[(6usize, 3u8)][..],
        &[(7, 1), (5, 0)][..],
        &[(7, 1), (5, 0)][..],
        &[(6, 0)][..],
    ] {
        for (row, bit) in keys {
            spec.bus.keys[*row] &= !(1 << bit);
        }
        for _ in 0..4 {
            spec.run(FRAME_T);
        }
        for (row, bit) in keys {
            spec.bus.keys[*row] |= 1 << bit;
        }
        for _ in 0..4 {
            spec.run(FRAME_T);
        }
    }
    let now = spec.bus.total_t();
    spec.bus.tape.as_mut().unwrap().play(now);
    for _ in 0..600 {
        spec.run(FRAME_T);
    }
    let ran_in_ram = (0x4000..=0xFFFFu16)
        .filter(|a| spec.bus.tracker.executed[spec.bus.phys_index(*a)])
        .count();
    assert!(
        ran_in_ram > 20,
        "Border Break's code has run: {ran_in_ram} addresses"
    );

    let notes = Notes::unattached();
    let out = asmexport::from_machine(&spec, &notes, None, "Border Break");
    assert!(
        out.text.contains(&asmexport::describe_rom("ROM", &rom)),
        "the header names the ROM it ran against"
    );
    let bytes = assemble(&asm, &out.text, "borderbreak");
    let memory: Vec<u8> = (0x4000..=0xFFFFu16).map(|a| spec.bus.peek_raw(a)).collect();
    assert_eq!(bytes.len(), memory.len());
    let first = bytes.iter().zip(&memory).position(|(a, b)| a != b);
    assert_eq!(
        first,
        None,
        "first difference at ${:04X}",
        first.unwrap_or(0) + 0x4000
    );
    eprintln!(
        "Border Break: {} instructions, {} bytes as data",
        out.instructions, out.data_bytes
    );
}

// The rebuilt games are moving into the 128K's spare banks, which are only
// ever seen at $C000-$FFFF, so the same address can hold two things and the
// assembler has to keep them apart: a bank of its own in a file of its own,
// and the bank written beside each of its labels in the map.
#[test]
fn the_python_assembler_puts_code_in_a_bank_of_its_own() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("no python3; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("zxrs-z80asm-bank-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin, map) = (
        dir.join("game.asm"),
        dir.join("game.bin"),
        dir.join("game.map"),
    );
    let assemble = |source: &str| {
        std::fs::write(&src, source).unwrap();
        std::process::Command::new("python3")
            .arg("tools/z80asm.py")
            .arg(&src)
            .arg(&bin)
            .arg("--map")
            .arg(&map)
            .arg("--mcp")
            .arg(env!("CARGO_BIN_EXE_mcp"))
            .output()
            .expect("python3 runs")
    };
    let run = assemble(
        "        ORG $8000\nstart:\n        JP far\n        BANK 3\n        ORG $C000\nfar:\n        \
         LD B,$33\n        RET\n        BANK\n        ORG $C000\nnear:\n        LD C,$00\n",
    );
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let main = std::fs::read(&bin).unwrap();
    let bank = std::fs::read(dir.join("game.bank3.bin")).unwrap();
    assert_eq!(&main[..3], &[0xC3, 0x00, 0xC0], "JP far, to $C000");
    assert_eq!(
        main.len(),
        0x4002,
        "$8000 to the end of near, at $C000 in the main image"
    );
    assert_eq!(&main[0x4000..], &[0x0E, 0x00], "near is in the main image");
    assert_eq!(bank.len(), 0x4000, "a whole bank");
    assert_eq!(&bank[..3], &[0x06, 0x33, 0xC9], "far is in bank 3");
    let map = std::fs::read_to_string(&map).unwrap();
    assert!(
        map.contains("far $C000 bank3\n") && map.contains("near $C000\n"),
        "{map}"
    );

    let run = assemble("        BANK 4\n        ORG $8000\n        NOP\n");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        !run.status.success() && said.contains("only seen from $C000"),
        "{said}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// The scripts in assemblies/ import the assembler and read what run returns
// as the main image, address by address. Banks came in by making that
// (bank, address) for every byte, which broke relocate.py with a KeyError
// at the first address it looked up; the banked bytes are kept beside it.
#[test]
fn the_python_assembler_gives_back_the_main_image_by_address_with_banks_beside_it() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("no python3; skipping");
        return;
    }
    let script = format!(
        "import sys; sys.path.insert(0, 'tools'); import z80asm\n\
         asm = z80asm.Assembler(z80asm.load_table({:?}))\n\
         src = ['        ORG $8000', '        LD B,$33', '        BANK 3', '        ORG $C000', \
                '        RET', '        BANK']\n\
         image, starts = asm.run(list(enumerate(src, 1)))\n\
         print(sorted(image.items()), sorted((b, sorted(m.items())) for b, m in asm.banked.items()))\n",
        env!("CARGO_BIN_EXE_mcp")
    );
    let run = std::process::Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 runs");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        said.trim(),
        "[(32768, 6), (32769, 51)] [(3, [(49152, 201)])]",
        "the main image by address, and bank 3's RET beside it"
    );
}
