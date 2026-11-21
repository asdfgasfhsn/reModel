//! Variants: the other releases of a title, as thin manifests over the
//! parent's (`crate::variants`, docs/variants.md). Each lists only what
//! differs from its parent: the chips it swaps (the sizes and hashes of its
//! own dumps) and the few per-game values that differed in practice. The
//! parent's controls, boards, shading and the rest are inherited; scenes and
//! tours are not.

use super::rom;
use crate::variants::{ChipSwap, Variant, VariantKind, VariantLink};
use crate::{Board, Recommended, Role};

/// The parent's own name in its version list ("Export", "Rev C"): what the
/// default set of a title with variants is. Only titles with variants.
pub static PARENT_LABELS: &[(&str, &str)] = &[
    ("topskatr", "Export"),
    ("vcop", "Rev B"),
    ("srallyc", "Rev C"),
    ("daytona", "Rev A"),
    ("lastbrnx", "Export, Rev A"),
    ("manxtt", "Deluxe, Rev D"),
    ("schamp", "Sonic Championship"),
    ("overrev", "Model 2C"),
];

const fn swap(parent: &'static str, chip: crate::RomFile) -> ChipSwap {
    ChipSwap { parent, chip }
}

pub static VARIANTS: &[Variant] = &[
    // The twin cabinet release of manxtt: its program (`epr-18822c..18825c`),
    // sound program (`epr-18826.30`) and first sample ROM (`mpr-18827.31`)
    // differ; it lacks the deluxe set's `epr-18742.7` and `mpr-18743.32`
    // (docs/games/manxtt.md, "`manxttc`"). Its link record is the parent's.
    Variant {
        id: "manxttc",
        parent: "manxtt",
        label: "Twin, Rev C",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        title: Some("Manx TT Superbike (Twin)"),
        swaps: &[
            swap(
                "epr-18822d.12",
                rom("epr-18822c.12", 0x20000, "d3a6910bf6efc138e0e40332219b90dea7d6ea56", Role::Program),
            ),
            swap(
                "epr-18823d.13",
                rom("epr-18823c.13", 0x20000, "6da5c071e3ce842a99f928f473d4ccf7165785ac", Role::Program),
            ),
            swap(
                "epr-18824d.14",
                rom("epr-18824c.14", 0x20000, "389cbf951ba606acb9ab7bff5cda85d9166e64ff", Role::Program),
            ),
            swap(
                "epr-18825d.15",
                rom("epr-18825c.15", 0x20000, "f6196e8da5e6579fe3fa5c24ab9538964c98e267", Role::Program),
            ),
            swap(
                "mpr-18863.31",
                rom("mpr-18827.31", 0x200000, "95275ed8315c044bfde2f23c10416f22627b34df", Role::Samples),
            ),
            swap(
                "epr-18824a.30",
                rom("epr-18826.30", 0x40000, "c3dd8a1324a4dc9b012bd9bf21d1f48578870f72", Role::AudioCpu),
            ),
        ],
        drop: &["epr-18742.7", "mpr-18743.32"],
        link: VariantLink::Parent,
        ..Variant::BASE
    },
    // The Japanese release of schamp: its program pair and the 1 MB `.7/.8`
    // pair differ; its idle wait is 0x2c bytes lower and its attract is the
    // title screen with the logo (docs/games/schamp.md, "`sfight`"). Its shading,
    // texture RAM mirroring and sound level are the parent's, as its controls.
    Variant {
        id: "sfight",
        parent: "schamp",
        label: "Sonic The Fighters",
        kind: VariantKind::Region,
        kind_confirmed: true,
        title: Some("Sonic The Fighters"),
        swaps: &[
            swap(
                "epr-19141.15",
                rom("epr-19001.15", 0x80000, "20718d985d14f4d2b1b8e982bfbebddd73cdb972", Role::Program),
            ),
            swap(
                "epr-19142.16",
                rom("epr-19002.16", 0x80000, "edcbf61122db568ccaa4c3106f507087c1740c9b", Role::Program),
            ),
            swap(
                "epr-19143.7",
                rom("epr-19003.7", 0x80000, "cbd55b7b7376ac2f67befaf4c43eef3727ba7b7f", Role::MainDataHigh),
            ),
            swap(
                "epr-19144.8",
                rom("epr-19004.8", 0x80000, "cf806501dbfa48d16cb7ed5f39a6146f734ba455", Role::MainDataHigh),
            ),
        ],
        attract_frame: Some(3500),
        idle_loops: Some(&[crate::IdleLoop { start: 0x11610, end: 0x1161c }]),
        ..Variant::BASE
    },
    // The 2B board's Over Rev: only the program pair differs (the zip holds both pairs);
    // every other chip is the 2C set's, in the same order (docs/games/overrevb.md).
    Variant {
        id: "overrevb",
        parent: "overrev",
        label: "Model 2B",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-20124a.15",
                rom("epr-19992b.15", 0x80000, "40d18ee284ea2e038f7e3d04db56e793ab3e3dd5", Role::Program),
            ),
            swap(
                "epr-20125a.16",
                rom("epr-19993b.16", 0x80000, "a718c32ca27ec1fb5ed2d7d3797ea7e906510a04", Role::Program),
            ),
        ],
        board: Some(Board::Model2B),
        // Its attract runs a few frames behind overrev's.
        attract_frame: Some(1720),
        recommended: Some(Recommended::DEFAULT),
        ..Variant::BASE
    },
    // Top Skater (Japan): the program pair, `topskatrj/epr-19753.15` and `.16` in the zip.
    Variant {
        id: "topskatrj",
        parent: "topskatr",
        label: "Japan",
        kind: VariantKind::Region,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-19755a.15",
                rom("epr-19753.15", 0x80000, "ca998de446c4c423db186696f3478f3daa4f8373", Role::Program),
            ),
            swap(
                "epr-19756a.16",
                rom("epr-19754.16", 0x80000, "f72f25cbd380918b919c11a7d2051948c8c484db", Role::Program),
            ),
        ],
        ..Variant::BASE
    },
    // Top Skater (USA): the same member names in `topskatru/`.
    Variant {
        id: "topskatru",
        parent: "topskatr",
        label: "USA",
        kind: VariantKind::Region,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-19755a.15",
                rom("epr-19753.15", 0x80000, "717ebf0ccd87128a24776e618cf15f07aaf48537", Role::Program),
            ),
            swap(
                "epr-19756a.16",
                rom("epr-19754.16", 0x80000, "a2329d09821900ec4f867caf1a93759085bd0a62", Role::Program),
            ),
        ],
        ..Variant::BASE
    },
    // Virtua Cop, Rev A: the `.12/.13` program pair.
    Variant {
        id: "vcopa",
        parent: "vcop",
        label: "Rev A",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-17166b.12",
                rom("epr-17166a.12", 0x20000, "478eec1e1d51a2ff63e8fd591528f0ca70df9310", Role::Program),
            ),
            swap(
                "epr-17167b.13",
                rom("epr-17167a.13", 0x20000, "5e95f3f901d7f87f8c9cbeb3a65cd1b74e9cc09b", Role::Program),
            ),
        ],
        ..Variant::BASE
    },
    // Sega Rally, Rev B. Its settings record is byte-identical to the parent's (checksum valid),
    // so the parent's link play data fits (docs/variants.md).
    Variant {
        id: "srallycb",
        parent: "srallyc",
        label: "Rev B",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-17888c.12",
                rom("epr-17888b.12", 0x80000, "9b293b430db14cfab35466d2f9a1e3f7e2df3143", Role::Program),
            ),
            swap(
                "epr-17889c.13",
                rom("epr-17889b.13", 0x80000, "9868d2b79255120abfdb7f9c0930a607aeef5363", Role::Program),
            ),
        ],
        link: VariantLink::Parent,
        ..Variant::BASE
    },
    // Sega Rally, the "Pro Drivin'" set (chips named `ic12.bin`, `ic13.bin`): different car
    // liveries and scenery, so a prototype by the screens and the chip names (*unconfirmed*:
    // nothing in the set says so). Same settings record.
    Variant {
        id: "srallyp",
        parent: "srallyc",
        label: "Pro Drivin' (prototype)",
        kind: VariantKind::Prototype,
        kind_confirmed: false,
        swaps: &[
            swap("epr-17888c.12", rom("ic12.bin", 0x80000, "7a37813f3d59b8e6e68220aa3120460b1ce5b33d", Role::Program)),
            swap("epr-17889c.13", rom("ic13.bin", 0x80000, "8477a05e44c1c9ee5e99eaa9d31cd9905ebf8c26", Role::Program)),
        ],
        link: VariantLink::Parent,
        ..Variant::BASE
    },
    // Daytona USA Turbo: a modified set (chips `turbo1.12`, `turbo2.13`; the program is 99.7%
    // the parent's), *unconfirmed* as a hack: the kind is the dat's name for it and its chip
    // names. Idle loops and settings record fit the parent's.
    Variant {
        id: "daytonat",
        parent: "daytona",
        label: "Turbo",
        kind: VariantKind::Hack,
        kind_confirmed: false,
        swaps: &[
            swap("epr-16722a.12", rom("turbo1.12", 0x20000, "daa75f38a11eb16b04550edf53e11f0eaf55cd3e", Role::Program)),
            swap("epr-16723a.13", rom("turbo2.13", 0x20000, "102ad6bf5fed4c9c407a9e82d85cff9f15db31c8", Role::Program)),
        ],
        link: VariantLink::Parent,
        ..Variant::BASE
    },
    Variant {
        id: "daytonata",
        parent: "daytona",
        label: "Turbo, Rev A",
        kind: VariantKind::Hack,
        kind_confirmed: false,
        swaps: &[
            swap(
                "epr-16722a.12",
                rom("dayturbo.12", 0x20000, "e29261de4344c99d82c9e494467605593cc776d8", Role::Program),
            ),
            swap(
                "epr-16723a.13",
                rom("dayturbo.13", 0x20000, "90b8f673a4ef88e7c1f6012b80823d3e756f9743", Role::Program),
            ),
        ],
        link: VariantLink::Parent,
        ..Variant::BASE
    },
    // Last Bronx, Japan Rev A. Its intro runs a few hundred frames differently (a SEGA logo
    // where the parent shows a fight at frame 1200).
    Variant {
        id: "lastbrnxj",
        parent: "lastbrnx",
        label: "Japan, Rev A",
        kind: VariantKind::Region,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-19061a.15",
                rom("epr-19046a.15", 0x80000, "e57320ac3abac54b7b5278596979746ed1856188", Role::Program),
            ),
            swap(
                "epr-19062a.16",
                rom("epr-19047a.16", 0x80000, "87214f285a7bf67fbd824f2190cb9b2daf408193", Role::Program),
            ),
        ],
        // The parent's attract frame (700) is on the warning text here; the fight demo (contact sheet
        // of a no-input run) is up by then.
        attract_frame: Some(1800),
        ..Variant::BASE
    },
    Variant {
        id: "lastbrnxu",
        parent: "lastbrnx",
        label: "USA, Rev A",
        kind: VariantKind::Region,
        kind_confirmed: true,
        swaps: &[
            swap(
                "epr-19061a.15",
                rom("epr-19059a.15", 0x80000, "c6b7a5788617faff6cf612a824b29a9474db87f3", Role::Program),
            ),
            swap(
                "epr-19062a.16",
                rom("epr-19060a.16", 0x80000, "a0904c97234f218caf489dc55e33082e453791a0", Role::Program),
            ),
        ],
        // The parent's attract frame (700) is on the SEGA logo here; the fight demo (contact sheet
        // of a no-input run) is up by then.
        attract_frame: Some(1500),
        ..Variant::BASE
    },
];
