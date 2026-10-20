//! Saying a shader in a host's terms while a title is running.
//!
//! Most of what a title hands the hardware is in its executable, and that is
//! read once when the executable is turned into a program. Some of it is not:
//! a title loads shaders from the data beside its executable, and those exist
//! only in a running title's memory. Reading the executable can never find one
//! of those however well the rest of this crate handles it.
//!
//! So a running program is given a way to ask. It hands over the words a title
//! handed the hardware and gets back what a driver takes.
//!
//! What comes back is one run of words rather than a shape both sides have to
//! agree on, because a shape shared across two languages is a thing that can be
//! got subtly wrong and that nothing checks. The run begins with three counts
//! and everything after them is read off in order.

use crate::{Kind, Translated, translate};

/// How many words come before the answer itself: how long the module is, how
/// many streams a shader fetches from, and how many pictures it reads.
pub const COUNTS: usize = 3;

/// How many words each stream takes: which fetch constant says where it is, how
/// far apart two vertices sit, how far into one this reads, and how much of it.
pub const WORDS_A_STREAM: usize = 4;

/// Says a shader in a host's terms, for a program with one to say while it is
/// running.
///
/// Returns how many words the answer takes, which is nought where the shader
/// could not be said. Where that is more than there was room for, nothing is
/// written and the caller is free to ask again with more.
///
/// # Safety
///
/// `from` must point at `count` readable words, and `into` at `room` writable
/// ones. Either may be null only where its count is nought.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenolith_say_a_shader(
    kind: u32,
    from: *const u32,
    count: u32,
    into: *mut u32,
    room: u32,
) -> u32 {
    let Some(kind) = Kind::from_word(kind) else {
        return 0;
    };
    let Ok(count) = usize::try_from(count) else {
        return 0;
    };
    if from.is_null() || count == 0 {
        return 0;
    }

    // SAFETY: the caller says these words are there to read.
    let words = unsafe { std::slice::from_raw_parts(from, count) };
    /* A title may hand a shader over with a header in front of it, which is
     * what a shader kept in the data beside the executable carries. The run
     * is read from where the shader begins rather than refused for what sits
     * before it. */
    let said = if let Ok(said) = translate(kind, words) {
        said
    } else {
        let Some(found) = crate::scan(words).into_iter().next() else {
            return 0;
        };
        let Some(inside) = words.get(found.at..found.at + found.words) else {
            return 0;
        };
        let Ok(said) = translate(kind, inside) else {
            return 0;
        };
        said
    };

    let answer = laid_out(&said);
    let Ok(length) = u32::try_from(answer.len()) else {
        return 0;
    };
    if length > room || into.is_null() {
        return length;
    }

    // SAFETY: the caller says there is room for this many words, and the line
    // above has established that the answer is no longer than that.
    let there = unsafe { std::slice::from_raw_parts_mut(into, answer.len()) };
    there.copy_from_slice(&answer);
    length
}

/// Lays a translation out as the run of words a running program reads.
fn laid_out(said: &Translated) -> Vec<u32> {
    let mut all = Vec::with_capacity(
        COUNTS + said.words.len() + said.streams.len() * WORDS_A_STREAM + said.pictures.len(),
    );

    all.push(u32::try_from(said.words.len()).unwrap_or_default());
    all.push(u32::try_from(said.streams.len()).unwrap_or_default());
    all.push(u32::try_from(said.pictures.len()).unwrap_or_default());
    all.extend_from_slice(&said.words);
    for stream in &said.streams {
        all.push(stream.slot);
        all.push(stream.stride);
        all.push(stream.offset);
        all.push(stream.words);
    }
    all.extend_from_slice(&said.pictures);
    all
}

#[cfg(test)]
mod tests {
    use super::{COUNTS, WORDS_A_STREAM, xenolith_say_a_shader};

    /// One a title hands over while it is running, which reading its executable
    /// does not find.
    const WORDS: &str = "\
        70956003 60091000 56000000 00000000 400fc200 12000000 00000000 2013c400 \
        22000000 3df80000 00263647 0000000d 05f81000 4026321a 0000040d 05f82000 \
        0006060a 00000c0d 70000000 0000001b e2000000 c8010000 181bc6b1 8b00ffff \
        2c100000 1800006c e2000000 c8040003 186c1b6c 8b00ffff c0130003 1ac5b0c6 \
        e0000103 c4280000 1ab16cc6 e1030003 c8070000 18bcb000 c1030000 c8030000 \
        18b01a00 c0000000 c8060000 1861bc00 c0000100 c801803e 00c5b01b 91002828 \
        c802803e 00c5b01b 91002929 c804803e 00c5b01b 91002a2a c808803e 00c5b01b \
        91002b2b c8038000 001a1a00 c2010100 c80f8001 00000000 c2020200 4e4a0000 \
        649bd3d7 177391ea";

    fn words() -> Vec<u32> {
        WORDS
            .split_whitespace()
            .map(|word| u32::from_str_radix(word, 16).expect("a hexadecimal word"))
            .collect()
    }

    #[test]
    fn asking_with_no_room_says_how_much_is_wanted() {
        let words = words();
        let wanted = unsafe {
            xenolith_say_a_shader(
                0,
                words.as_ptr(),
                u32::try_from(words.len()).expect("a length"),
                std::ptr::null_mut(),
                0,
            )
        };
        assert!(wanted > 0, "a shader a title hands over can be said");
    }

    #[test]
    fn what_comes_back_reads_as_the_counts_say() {
        let words = words();
        let count = u32::try_from(words.len()).expect("a length");
        let wanted =
            unsafe { xenolith_say_a_shader(0, words.as_ptr(), count, std::ptr::null_mut(), 0) };

        let mut room = vec![0u32; wanted as usize];
        let written =
            unsafe { xenolith_say_a_shader(0, words.as_ptr(), count, room.as_mut_ptr(), wanted) };
        assert_eq!(written, wanted);

        let module = room[0] as usize;
        let streams = room[1] as usize;
        let pictures = room[2] as usize;
        assert_eq!(
            COUNTS + module + streams * WORDS_A_STREAM + pictures,
            room.len(),
            "the counts account for every word that came back"
        );
        assert_eq!(room[COUNTS], 0x0723_0203, "a module begins as one does");
    }

    #[test]
    fn words_that_are_not_a_shader_are_refused_rather_than_answered() {
        let nonsense = [0u32; 12];
        let said =
            unsafe { xenolith_say_a_shader(0, nonsense.as_ptr(), 12, std::ptr::null_mut(), 0) };
        assert_eq!(said, 0);
    }
}

/// A pixel shader a title handed over that uses the four lane dot, which the
/// translator once refused. It must translate, and a validator that is present
/// must accept the result.
#[cfg(test)]
mod a_four_lane_dot_shader {
    use crate::{Kind, translate};

    #[test]
    #[ignore = "needs spirv-val"]
    fn translates_and_the_validator_accepts_it() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        const W: [u32; 30] = [
            0x0015_3002,
            0x0000_1200,
            0xc400_0000,
            0x0000_4005,
            0x0000_2200,
            0x0000_0000,
            0x1018_1001,
            0x1f1f_fff8,
            0x0000_4000,
            0x1028_1001,
            0x1f1f_ffc7,
            0x0000_4000,
            0x1008_1001,
            0x1f1f_fe3f,
            0x0000_4000,
            0xc807_0000,
            0x00be_6d00,
            0x8001_fe00,
            0xa880_0000,
            0x0000_0000,
            0xc200_00ff,
            0xc88d_c000,
            0x00b2_1a1b,
            0xab00_fe00,
            0xc802_8000,
            0x0012_c000,
            0x9000_ff00,
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
        ];
        let translated = translate(Kind::Pixel, &W)
            .unwrap_or_else(|refused| panic!("translation refused: {refused:?}"));
        let bytes: Vec<u8> = translated
            .words
            .iter()
            .copied()
            .flat_map(u32::to_le_bytes)
            .collect();
        let mut running = Command::new("spirv-val")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spirv-val could not be run, so this check could not run");
        running
            .stdin
            .take()
            .expect("spirv-val took no input")
            .write_all(&bytes)
            .expect("writing to spirv-val");
        let done = running.wait_with_output().expect("waiting for spirv-val");
        assert!(
            done.status.success(),
            "spirv-val refused the module: {}",
            String::from_utf8_lossy(&done.stderr)
        );
    }
}

/// Two shaders a title draws a scene with: a pixel shader using comparisons,
/// the predicate push and pop family, the four lane dot, and a subtraction
/// emitted as an integer one; and a vertex shader carrying a third thing on.
/// Both must translate, and a validator that is present must accept them.
#[cfg(test)]
mod two_scene_shaders {
    use crate::{Kind, translate};

    fn accepted(label: &str, kind: Kind, words: &[u32]) {
        let said =
            translate(kind, words).unwrap_or_else(|refused| panic!("{label}: refused {refused:?}"));
        let bytes: Vec<u8> = said
            .words
            .iter()
            .copied()
            .flat_map(u32::to_le_bytes)
            .collect();
        let path = std::env::temp_dir().join(format!("xenolith_{label}.spv"));
        std::fs::write(&path, &bytes).expect("write the module");
        let checked = std::process::Command::new("spirv-val")
            .arg(&path)
            .output()
            .expect("spirv-val could not be run, so this check could not run");
        assert!(
            checked.status.success(),
            "{label}: spirv-val rejected: {}",
            String::from_utf8_lossy(&checked.stderr)
        );
    }

    #[test]
    #[ignore = "needs spirv-val"]
    #[allow(clippy::too_many_lines)]
    fn the_scene_pixel_shader_is_accepted() {
        static WORDS: &[u32] = &[
            0x0009_600c,
            0x6012_1000,
            0x5600_0000,
            0x0000_6018,
            0x601e_1000,
            0x1000_0909,
            0x0000_6024,
            0x602a_5600,
            0x1000_0000,
            0x0024_6030,
            0x6036_1000,
            0x5600_0024,
            0x0000_603c,
            0x6042_5600,
            0x1000_0900,
            0x0090_6048,
            0x604e_1000,
            0x5600_0000,
            0x0240_6054,
            0x605a_1000,
            0x5600_0540,
            0x0002_6060,
            0x6066_5600,
            0x1000_0000,
            0x0240_606c,
            0x6072_1000,
            0x5600_0250,
            0x0000_6078,
            0x607e_5600,
            0x1000_0000,
            0x0000_3084,
            0x0000_5600,
            0xc400_0000,
            0x0000_1087,
            0x0000_2200,
            0x0000_0000,
            0x1000_3001,
            0x1f1f_f688,
            0x0000_4000,
            0xc80c_0200,
            0x0006_ac00,
            0xc601_0000,
            0x084f_0003,
            0x0000_00cb,
            0xe103_0200,
            0xc80f_0207,
            0x00c6_6c00,
            0xac00_fe03,
            0xc804_0200,
            0x00c6_1b00,
            0x45fe_0700,
            0xc808_0206,
            0x006c_c600,
            0x55fe_0000,
            0xc803_0200,
            0x1c6d_c66d,
            0x2b28_2800,
            0xc804_0200,
            0x18b1_1b6c,
            0x2b28_fe00,
            0xc80c_0203,
            0x1886_7100,
            0xc600_0100,
            0xc803_0203,
            0x18c7_b200,
            0xc601_0000,
            0xc808_0200,
            0x181b_b100,
            0xc103_0300,
            0x2820_0302,
            0x1800_001b,
            0xe202_0200,
            0xc808_0200,
            0x18c6_b100,
            0xc103_0300,
            0x2880_0002,
            0x1800_001b,
            0xe202_0200,
            0xc808_0200,
            0x186c_1b00,
            0xc103_0000,
            0xc808_0206,
            0x001b_1b00,
            0xd406_0000,
            0x2040_0202,
            0x1900_006c,
            0xe202_0282,
            0x7c80_0602,
            0x0000_001b,
            0xe202_0206,
            0x2408_2001,
            0x9f1f_fffb,
            0x8000_4000,
            0xa840_0202,
            0x1800_0080,
            0xe202_02ff,
            0x8080_0602,
            0x0000_001b,
            0xe202_0206,
            0xc80c_0200,
            0x1871_1bac,
            0x2b28_ff00,
            0xec08_2001,
            0x9f1f_ffdf,
            0x8000_4000,
            0xc802_0204,
            0x181b_c600,
            0xc601_0000,
            0xc805_0204,
            0x181a_c700,
            0xc601_0000,
            0xc80c_0203,
            0x18db_ac00,
            0xc600_0100,
            0xc801_0203,
            0x18c6_b100,
            0xc600_0100,
            0xc801_0202,
            0x18c6_6c00,
            0xc103_0400,
            0x1401_0202,
            0x186c_6c6c,
            0xa702_fe03,
            0x0c48_0303,
            0x181b_6cb1,
            0xe103_0203,
            0xa81c_0203,
            0x1806_6c41,
            0xa703_feff,
            0x1401_0204,
            0x18c6_1bb1,
            0xe104_0304,
            0xc801_0202,
            0x186c_6c6c,
            0xac04_fe02,
            0x0c44_0302,
            0x18c6_6cc6,
            0xe002_0203,
            0xc808_0206,
            0x001b_c600,
            0xd406_0300,
            0x2010_0202,
            0x1900_006c,
            0xe202_0282,
            0x7c80_0602,
            0x0000_001b,
            0xe202_0206,
            0x2408_2001,
            0x9f1f_fffb,
            0x8000_4000,
            0xa810_0202,
            0x1800_00c0,
            0xe202_02ff,
            0x8080_0602,
            0x0000_001b,
            0xe202_0206,
            0xc808_0200,
            0x186c_1bb1,
            0x2b28_fe00,
            0xb014_0602,
            0x18c6_6c41,
            0xc002_0228,
            0x1424_0606,
            0x181b_1bc6,
            0xe200_0000,
            0x9008_20c1,
            0x9f1f_ffdf,
            0x8000_4000,
            0xc805_0204,
            0x186e_6c00,
            0xc606_0100,
            0xc80c_0203,
            0x18c6_4600,
            0xc601_0600,
            0xc80c_0203,
            0x1846_db00,
            0xc104_0300,
            0x2810_0202,
            0x1800_001b,
            0xe202_0203,
            0x2848_0304,
            0x186c_b1c6,
            0xe600_0103,
            0x1401_0204,
            0x181b_6c1b,
            0xe601_0004,
            0x0c18_0303,
            0x186c_c66c,
            0xe103_0302,
            0xc809_0203,
            0x1818_6c00,
            0x8703_fe00,
            0xa819_0203,
            0x186d_6f41,
            0xe104_03ff,
            0xc801_0202,
            0x186c_6c6c,
            0xac03_fe02,
            0xc804_0202,
            0x18c6_6c00,
            0xc002_0200,
            0xc808_0206,
            0x001b_1b00,
            0xd406_0300,
            0x2020_0202,
            0x1900_006c,
            0xe202_0282,
            0x7c80_0602,
            0x0000_001b,
            0xe202_0206,
            0x0c08_2001,
            0x9f1f_fffb,
            0x8000_4000,
            0xa820_0202,
            0x1800_0080,
            0xe202_02ff,
            0x8080_0602,
            0x0000_001b,
            0xe202_0206,
            0xc808_0200,
            0x186c_1bb1,
            0x2b28_ff00,
            0x0c08_2001,
            0x9f1f_fffb,
            0x8000_4000,
            0xc801_0203,
            0x18c6_1b00,
            0xc601_0000,
            0xc804_0200,
            0x181b_6c00,
            0xc600_0100,
            0xc804_0200,
            0x18c6_6c00,
            0xc100_0300,
            0x2810_0302,
            0x1800_00c6,
            0xe202_0200,
            0x1406_0204,
            0x181b_61c6,
            0xe104_0302,
            0x0426_0204,
            0x18c1_6cb1,
            0xa704_fe02,
            0xa816_0204,
            0x186c_c1c0,
            0xe104_04ff,
            0xc801_0202,
            0x18c6_6c6c,
            0xac04_fe02,
            0xc808_0203,
            0x18b1_6c00,
            0xc002_0200,
            0xc808_0206,
            0x001b_b100,
            0xd406_0400,
            0x2040_0002,
            0x1900_006c,
            0xe202_0282,
            0x7c80_0602,
            0x0000_001b,
            0xe202_0206,
            0x0408_2001,
            0x9f1f_fffb,
            0x8000_4000,
            0xa840_0002,
            0x1800_0080,
            0xe202_02fd,
            0x8080_0602,
            0x0000_001b,
            0xe202_0206,
            0x1418_0505,
            0x186c_6c1b,
            0xe200_0000,
            0xc804_0205,
            0x186c_6cb1,
            0x2b28_fd00,
            0xb022_0506,
            0x186c_6c80,
            0xc200_0028,
            0x1008_20a1,
            0x9f1f_ffdf,
            0x8000_4000,
            0x3808_20a1,
            0x9f1f_feff,
            0x8000_4000,
            0x1008_20c1,
            0x9f1f_fffb,
            0x8000_4000,
            0xc802_0204,
            0x181b_b100,
            0xc601_0500,
            0xc804_0204,
            0x18b1_b100,
            0xc605_0100,
            0xc801_0205,
            0x18c6_c600,
            0xc601_0500,
            0x1408_0200,
            0x18c6_6c1b,
            0xe605_0103,
            0x0488_0800,
            0x181b_6cc6,
            0xe100_0500,
            0x284b_0009,
            0x1816_c01b,
            0xe104_0300,
            0xa824_0209,
            0x181b_c641,
            0xe104_00ff,
            0xa84f_0209,
            0x180d_6c82,
            0xa709_feff,
            0xa81f_0209,
            0x1870_3ec0,
            0xe104_09ff,
            0xc807_0208,
            0x18cf_6cb4,
            0xac09_fe02,
            0xc804_0202,
            0x1822_c600,
            0x8f08_fe00,
            0xc808_0206,
            0x001b_b100,
            0xd406_0900,
            0x2010_0202,
            0x1900_006c,
            0xe202_0282,
            0x7c80_0602,
            0x0000_001b,
            0xe202_0206,
            0x1410_0502,
            0x1800_00b1,
            0xe202_0200,
            0x1008_20a1,
            0x9f1f_fffb,
            0x8000_4000,
            0xa810_0202,
            0x1800_00c0,
            0xe202_02ff,
            0x8080_0602,
            0x0000_001b,
            0xe202_0206,
            0xc801_0200,
            0x18b1_6c6c,
            0x2b28_fd00,
            0x1410_0502,
            0x1800_006c,
            0xe202_0206,
            0x0408_0001,
            0x9f1f_ffdf,
            0x8000_4000,
            0x1008_20a1,
            0x9f1f_ffdf,
            0x8000_4000,
            0xc808_0200,
            0x181b_6c00,
            0xc601_0000,
            0x1401_0200,
            0x186c_b1c6,
            0xe600_0104,
            0x0c41_0000,
            0x186c_b1c6,
            0xe100_0303,
            0x1405_0200,
            0x186e_6cc6,
            0xa700_fe02,
            0x0444_0200,
            0x181b_c66c,
            0xe100_0002,
            0xa818_0000,
            0x18b1_6c41,
            0xe104_00ff,
            0xc803_0202,
            0x181a_6c6d,
            0xac00_fe00,
            0xc802_0202,
            0x18c6_b100,
            0xc002_0200,
            0xc801_0202,
            0x186c_b1b1,
            0xab02_ff02,
            0xc800_0202,
            0x001b_6c00,
            0xd606_0200,
            0xc801_0202,
            0x186c_1b00,
            0xc102_0200,
            0xa820_0202,
            0x1800_0080,
            0xe202_02fe,
            0xc801_0202,
            0x18b1_1b00,
            0xc002_0700,
            0xc801_0202,
            0x1cb1_1b6c,
            0xeb02_0702,
            0x4c12_0202,
            0x186c_6c6c,
            0xe202_0202,
            0xc801_0202,
            0x186c_1b00,
            0xc102_0700,
            0x1487_0707,
            0x186c_c0b1,
            0xe102_0702,
            0xc80f_8000,
            0x0000_0000,
            0xc207_0700,
            0x4e4a_0000,
            0xc6d2_b19b,
            0x5c1d_1d0e,
        ];
        accepted("ac6_scene_pixel", Kind::Pixel, WORDS);
    }

    #[test]
    #[ignore = "needs spirv-val"]
    fn the_scene_vertex_shader_is_accepted() {
        static WORDS: &[u32] = &[
            0xf255_6004,
            0x600a_1000,
            0x5600_0000,
            0x0000_1010,
            0x0000_5600,
            0xc200_0000,
            0x0000_4011,
            0x0000_1200,
            0xc400_0000,
            0x0000_3015,
            0x0000_2200,
            0x0000_0000,
            0x3df8_3000,
            0x0026_3ec8,
            0x0000_000d,
            0x05f8_1000,
            0x4026_321a,
            0x0000_040d,
            0x25f8_2000,
            0x0026_3688,
            0x0000_080d,
            0x05f8_0000,
            0x4006_060a,
            0x0000_0c0d,
            0x7000_0000,
            0x0000_00c6,
            0xe200_0003,
            0xc804_0003,
            0x18c6_c6b1,
            0x8b03_ffff,
            0x2c40_0300,
            0x1800_00c6,
            0xe200_0003,
            0xc804_0003,
            0x18c6_1b6c,
            0x8b03_ffff,
            0xc013_0304,
            0x1ab0_b0c6,
            0xe003_0103,
            0xc428_0303,
            0x1ab1_6cc6,
            0xe104_0303,
            0xc807_0003,
            0x18bc_b000,
            0xc104_0300,
            0xc803_0003,
            0x18b0_1a00,
            0xc003_0300,
            0xc803_0003,
            0x186d_b000,
            0xc003_0100,
            0xc801_803e,
            0x00b0_b01b,
            0x9103_2828,
            0xc802_803e,
            0x00b0_b01b,
            0x9103_2929,
            0xc804_803e,
            0x00b0_b01b,
            0x9103_2a2a,
            0xc808_803e,
            0x00b0_b01b,
            0x9103_2b2b,
            0xc803_8000,
            0x001a_1a00,
            0xc201_0100,
            0xc80f_8002,
            0x0000_0000,
            0xc200_0000,
            0xc80f_8001,
            0x0000_0000,
            0xc202_0200,
            0x4e4a_0000,
            0x1207_f124,
            0xf795_419d,
        ];
        accepted("ac6_scene_vertex", Kind::Vertex, WORDS);
    }
}
