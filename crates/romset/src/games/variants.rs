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
    ("stcc", "Latest"),
    ("vf2", "Rev 2.1"),
    ("vstriker", "Rev A"),
    ("von", "USA"),
    ("indy500", "Twin, newer"),
    ("zerogun", "Export"),
    ("dynamcop", "Dynamite Cop"),
    ("doa", "Model 2B"),
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
        link: VariantLink::Parent,
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
        // The parent's attract frame (700) is on the warning text screen here; the fight demo (contact sheet
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
        // The parent's attract frame (700) is on the SEGA logo screen here; the fight demo (contact sheet
        // of a no-input run) is up by then.
        attract_frame: Some(1500),
        ..Variant::BASE
    },
    // Sega Touring Car Championship, Rev A: the program pair (`epr-19272a.15/19273a.16`); DSB1 and sound chips are the parent's.
    Variant {
        id: "stcca",
        parent: "stcc",
        label: "Rev A",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-19570.15", rom("epr-19272a.15", 0x80000, "e465967c784de18caaaac77e164796e9779f576a", Role::Program)),
            swap("epr-19571.16", rom("epr-19273a.16", 0x80000, "142bcd53fa6632fcc866bbda817aa83470111ef1", Role::Program)),
        ],
        // The wait for vblank moved with the program: the same loop as the parent's, at 0x73424.
        idle_loops: Some(&[crate::IdleLoop { start: 0x73424, end: 0x7343c }]),
        // The settings record is `SEGASTCa`, not the parent's (docs/games/stcc.md).
        link: VariantLink::Own(&super::link::STCCA),
        ..Variant::BASE
    },
    // Sega Touring Car Championship, Rev B (`epr-19272b.15/19273b.16`).
    Variant {
        id: "stccb",
        parent: "stcc",
        label: "Rev B",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-19570.15", rom("epr-19272b.15", 0x80000, "5026e28b9d8267492bd0d9746d64526540a001da", Role::Program)),
            swap("epr-19571.16", rom("epr-19273b.16", 0x80000, "3f22f13a3baa46f93cb40e8af9534afaa57ead9c", Role::Program)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x73574, end: 0x7358c }]),
        link: VariantLink::Own(&super::link::STCCB),
        ..Variant::BASE
    },
    // Virtua Fighter 2, the original release: four program chips (`epr-17568.12`, `17569.13`, `17562.14`, `17563.15`).
    Variant {
        id: "vf2o",
        parent: "vf2",
        label: "Original",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-18385.12", rom("epr-17568.12", 0x20000, "4ed907bbfc1a47e51c9cc11f55645752574adaef", Role::Program)),
            swap("epr-18386.13", rom("epr-17569.13", 0x20000, "95efb3eeaf95fb5f79ddae4ef20e2211b07f8d30", Role::Program)),
            swap("epr-18387.14", rom("epr-17562.14", 0x20000, "2f862a7099aa757ee1f2ad8245eb4f8f4fdfb7bc", Role::Program)),
            swap("epr-18388.15", rom("epr-17563.15", 0x20000, "b1ca3d4d3568c1652dcd8e546ffff23a4a21a699", Role::Program)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x10a74, end: 0x10a80 }]),
        ..Variant::BASE
    },
    // Virtua Fighter 2, Rev A: four program chips (the `.ic12`..`.ic15` socket names).
    Variant {
        id: "vf2a",
        parent: "vf2",
        label: "Rev A",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-18385.12", rom("epr-17568a.ic12", 0x20000, "04df1eb9cf094d8dc5118b95028b544b47d5d328", Role::Program)),
            swap("epr-18386.13", rom("epr-17569a.ic13", 0x20000, "260c762d7853fb1d6f894d4dd954d82dfbc92d2d", Role::Program)),
            swap("epr-18387.14", rom("epr-17562a.ic14", 0x20000, "1e9d3f09821596d3560bf54f6323ba295ee430d8", Role::Program)),
            swap("epr-18388.15", rom("epr-17563a.ic15", 0x20000, "846fa1435a1a5c9f7f9690e1c810ca89008d1626", Role::Program)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x10a98, end: 0x10aa4 }]),
        ..Variant::BASE
    },
    // Virtua Fighter 2, Rev B: four program chips.
    Variant {
        id: "vf2b",
        parent: "vf2",
        label: "Rev B",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-18385.12", rom("epr-17568b.12", 0x20000, "01d46313148ce509fa5641fb07a3f840c00886ac", Role::Program)),
            swap("epr-18386.13", rom("epr-17569b.13", 0x20000, "ba2e0ac8b31955fed237ba9a5eda9fa14d1db11f", Role::Program)),
            swap("epr-18387.14", rom("epr-17562b.14", 0x20000, "a7162d9c39d601ac92310c8cf2ae388647a5295a", Role::Program)),
            swap("epr-18388.15", rom("epr-17563b.15", 0x20000, "b9b1f3c68c53a86dfa3cbc85fcb9150546c13f23", Role::Program)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x10b3c, end: 0x10b48 }]),
        ..Variant::BASE
    },
    // Virtua Striker, the original revision: the four program chips and the `.7/.8` pair without the parent's `a` suffix.
    Variant {
        id: "vstrikro",
        parent: "vstriker",
        label: "Original",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("ep18068a.15", rom("ep18068.15", 0x20000, "3ba34bd467e11e768eda95ff345f5993fb9d6bca", Role::Program)),
            swap("ep18069a.16", rom("ep18069.16", 0x20000, "84bf16fc2a441cb724f4bc635a4c4209c240cfbf", Role::Program)),
            swap("ep18066a.13", rom("ep18066.13", 0x20000, "0ff20aa3e030df869767bb9614565acc9f3fe3b1", Role::Program)),
            swap("ep18067a.14", rom("ep18067.14", 0x20000, "d5eff8aff37fb0ef3c7f9d8bfca8460213b0f0a7", Role::Program)),
            swap("ep18070a.7", rom("ep18070.7", 0x80000, "731452284c45329701258ee9fb8b7df6514fbba1", Role::MainData)),
            swap("ep18071a.8", rom("ep18071.8", 0x80000, "c678f1f42de86cc968c3f823994d36c74b2e55fd", Role::MainData)),
        ],
        // The wait for vblank, 0x80 above the parent's (the run's hot spots).
        idle_loops: Some(&[crate::IdleLoop { start: 0x3dd4, end: 0x3de8 }]),
        ..Variant::BASE
    },
    // Virtual-On, Japan: the program pair `epr-18664b.15/18665b.16` for the parent's `vo-prog0.usa/vo-prog1.usa`.
    Variant {
        id: "vonj",
        parent: "von",
        label: "Japan",
        kind: VariantKind::Region,
        kind_confirmed: true,
        swaps: &[
            swap("vo-prog0.usa", rom("epr-18664b.15", 0x80000, "f3bcae9898c7d656eccb4d2546c9bb93daaefbb7", Role::Program)),
            swap("vo-prog1.usa", rom("epr-18665b.16", 0x80000, "73f2a19a519ced8e0a1ab5cf69a4bf9d9841e288", Role::Program)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x28e2c, end: 0x28e40 }]),
        // The SRAM settings record (header, checksum and defaults) is the parent's.
        link: VariantLink::Parent,
        ..Variant::BASE
    },
    // Indianapolis 500, the Deluxe cabinet: the program pair, the `.7/.8` pair and the sound program differ.
    Variant {
        id: "indy500d",
        parent: "indy500",
        label: "Deluxe, Rev A",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-18598a.15", rom("epr-18251a.15", 0x80000, "e60a4814b54b76c7c0a4d9cf2b093c577c2f6ecf", Role::Program)),
            swap("epr-18599a.16", rom("epr-18252a.16", 0x80000, "8fc9244fd0eaf93d016f4494604e5a70bf1f7303", Role::Program)),
            swap("epr-18596.7", rom("epr-18345.7", 0x80000, "6bbbae53e2f56ab1007f37fdd5eb66dda4828c28", Role::MainDataHigh)),
            swap("epr-18597.8", rom("epr-18346.8", 0x80000, "425ca92d75054a17313732a9d11bbb31bea17a38", Role::MainDataHigh)),
            swap("epr-18600.31", rom("epr-18253.31", 0x40000, "4a3037b69c4835ef16a20c5573de32a862f0b13e", Role::AudioCpu)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x345d4, end: 0x345e8 }]),
        // The parent's attract frame (1500) is the title logo here; the demo race runs from 1650.
        attract_frame: Some(2250),
        link: VariantLink::Own(&super::link_variants::INDY500D),
        ..Variant::BASE
    },
    // Indianapolis 500, the older Twin release: the program pair, the `.7/.8` pair and the sound program differ.
    Variant {
        id: "indy500to",
        parent: "indy500",
        label: "Twin, older",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        swaps: &[
            swap("epr-18598a.15", rom("epr-18254a.15", 0x80000, "0bff35fc1d892aaffbf1a3965bf3109c54839f4b", Role::Program)),
            swap("epr-18599a.16", rom("epr-18255a.16", 0x80000, "299e87f8ec7bdefa6f94f4ab65e29e91f290611e", Role::Program)),
            swap("epr-18596.7", rom("epr-18389.7", 0x80000, "ef10bb0ffcb1bbcf4672bb5f705a27679a793764", Role::MainDataHigh)),
            swap("epr-18597.8", rom("epr-18390.8", 0x80000, "b23cfe45c363d616a65decd57aeb8ae61d5370e9", Role::MainDataHigh)),
            swap("epr-18600.31", rom("epr-18391.31", 0x40000, "36fed8a9eeb34968b2852ea8fc9198427f0d27c6", Role::AudioCpu)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x2e1c4, end: 0x2e1d8 }]),
        // The older release is on its warning screen until about frame 1500 and takes a start press
        // only after the ranking screen is up (the parent's at 1250 is ignored: it stays on PRESS START
        // BUTTON): the same script with the start press at 1900.
        coin_start: Some("coin1@1100-1104,coin1@1120-1124,start@1900-1904,gas=0xff@2050-2060,gas=0xff@2250-2260,gas=0xff@2450-2460,gas=0xff@2650-99999"),
        attract_frame: Some(2250),
        link: VariantLink::Own(&super::link_variants::INDY500TO),
        ..Variant::BASE
    },
    // Daytona USA '93 Edition: ten chips differ (program pair, main data `.8/.9` and `.6/.7`, polygons `.19/.23`, textures `.26/.27`), at other sizes.
    Variant {
        id: "dayton93",
        parent: "daytona",
        label: "'93 Edition",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        title: Some("Daytona USA '93 Edition"),
        swaps: &[
            swap("epr-16722a.12", rom("epr-16530a.12", 0x20000, "b98a1faabb4f1eff707a94c32224c7820f259874", Role::Program)),
            swap("epr-16723a.13", rom("epr-16531a.13", 0x20000, "779734ba536db67e14760d52e8d8d7db07816481", Role::Program)),
            swap("mpr-16808.8", rom("epr-16526.8", 0x200000, "f505910394d41a9ffecfdea7b45ef25b21469b7a", Role::MainData)),
            swap("mpr-16809.9", rom("epr-16527.9", 0x200000, "1bf3aec88ef9fb40bde054f5f0b884bf715cbcc8", Role::MainData)),
            swap("epr-16724a.6", rom("epr-16534a.6", 0x100000, "814004e3426b5638e9c8b226594f4f2a9138ffed", Role::MainData)),
            swap("epr-16725a.7", rom("epr-16535.7", 0x100000, "607bc0f6c478c3d83ce81f34b7f69997361f906f", Role::MainData)),
            swap("mpr-16772.19", rom("epr-16646.19", 0x80000, "6bcae009e8264bf038fe5d4bc436ec4fc1674831", Role::Polygons)),
            swap("mpr-16771.23", rom("epr-16645.23", 0x80000, "488fd6c0246752b7d9c25d7ba6cdc5b3911d1836", Role::Polygons)),
            swap("mpr-16769.26", rom("epr-16516.26", 0x200000, "a2ed7c586dfcb7980190b7057fa5366239035fe8", Role::Textures)),
            swap("mpr-16770.27", rom("epr-16517.27", 0x200000, "99be9c5d9d99f7016199ffa8404fc471d09e360d", Role::Textures)),
        ],
        // Only the first of the parent's two frame-wait loops exists in this program (the second is a
        // function this older program lacks: its bytes are not in the chips); the first moved.
        idle_loops: Some(&[crate::IdleLoop { start: 0x12b0, end: 0x12bc }]),
        // Its textures are its own chips: the parent's pack would miss.
        own_textures: true,
        ..Variant::BASE
    },
    // Daytona USA with adverts for the Saturn: the program pair and `.6/.7`.
    Variant {
        id: "daytonas",
        parent: "daytona",
        label: "Saturn Ads",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        title: Some("Daytona USA (Saturn Ads)"),
        swaps: &[
            swap("epr-16722a.12", rom("epr-17965.12", 0x20000, "3c337d12f4e12141b412a7289df46f44c66964b2", Role::Program)),
            swap("epr-16723a.13", rom("epr-17966.13", 0x20000, "2df03455a00ae7066c30bace5c2b81581529e6f4", Role::Program)),
            swap("epr-16724a.6", rom("epr17967.6", 0x80000, "a716646be6be0b87a550cb88b40e62a5c203ffdf", Role::MainData)),
            swap("epr-16725a.7", rom("epr17968.7", 0x80000, "9eb43314f3dc6acbbe0aa991d7a5fa44afe9cdd0", Role::MainData)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x13a8, end: 0x13b4 }, crate::IdleLoop { start: 0x1bda8, end: 0x1bdb4 }]),
        // The program's own defaults as the game wrote them to the I/O board's store
        // (version 4; a record of the parent's version is not accepted), with the link ID at
        // byte 0x0B set to SINGLE and the CRC-16 to match, as the parent's (docs/games/daytona.md).
        backup_ram: Some(&[(
            0,
            &[
                0x53, 0x45, 0x47, 0x41, 0x40, 0x82, 0x04, 0x00, 0xb2, 0x3e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
                0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x14, 0x1c, 0x00, 0x01,
                0x00, 0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            ],
        )]),
        link: VariantLink::Own(&super::link_variants::DAYTONAS),
        ..Variant::BASE
    },
    // Daytona USA Special Edition: the program pair and `.6/.7`.
    Variant {
        id: "daytonase",
        parent: "daytona",
        label: "Special Edition",
        kind: VariantKind::Revision,
        kind_confirmed: true,
        title: Some("Daytona USA Special Edition"),
        swaps: &[
            swap("epr-16722a.12", rom("epr-17369a.12", 0x20000, "16e9fd25670ce4eda378df402066e3d9652210b1", Role::Program)),
            swap("epr-16723a.13", rom("epr-17370a.13", 0x20000, "26eff5a07f6906e1ad20cd264ce6e25a9068ea2b", Role::Program)),
            swap("epr-16724a.6", rom("epr-13731.6", 0x80000, "412d4db62436746da8d0d55ccf2016d14c05153c", Role::MainData)),
            swap("epr-16725a.7", rom("epr-13732.7", 0x80000, "0c7502c2fe5a64db7e6020457b9f8e47f2c9af0e", Role::MainData)),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x13a8, end: 0x13b4 }, crate::IdleLoop { start: 0x1bd38, end: 0x1bd44 }]),
        // The program's own defaults as the game wrote them to the I/O board's store
        // (version 3; a record of the parent's version is not accepted), with the link ID at
        // byte 0x0B set to SINGLE and the CRC-16 to match, as the parent's (docs/games/daytona.md).
        backup_ram: Some(&[(
            0,
            &[
                0x53, 0x45, 0x47, 0x41, 0x40, 0x82, 0x03, 0x00, 0x04, 0xad, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
                0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x14, 0x1c, 0x00, 0x01,
                0x00, 0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            ],
        )]),
        link: VariantLink::Own(&super::link_variants::DAYTONASE),
        ..Variant::BASE
    },
    // Zero Gunner, Japan (Model 2B): the program pair `epr-20290.15/20291.16`.
    Variant {
        id: "zerogunj",
        parent: "zerogun",
        label: "Japan",
        kind: VariantKind::Region,
        kind_confirmed: true,
        swaps: &[
            swap("epr-20439.15", rom("epr-20290.15", 0x80000, "812ab45cc9e2920e74e58937d1826774f3f54183", Role::Program)),
            swap("epr-20440.16", rom("epr-20291.16", 0x80000, "a7216914ee7535fa1856cb19bc05c89948a93c89", Role::Program)),
        ],
        // The wait for vblank is code copied to work RAM, as the parent's, 0x50 higher.
        idle_loops: Some(&[crate::IdleLoop { start: 0x5490a0, end: 0x5490b4 }]),
        ..Variant::BASE
    },
    // Dynamite Deka 2 (Japan), Model 2A: the program pair `.12/.13`; `.14/.15` are shared.
    Variant {
        id: "dyndeka2",
        parent: "dynamcop",
        label: "Dynamite Deka 2",
        kind: VariantKind::Region,
        kind_confirmed: true,
        title: Some("Dynamite Deka 2"),
        swaps: &[
            swap("epr-20930.12", rom("epr-20922.12", 0x80000, "4076998fc600c1df3bb5ef48d42681c01e651495", Role::Program)),
            swap("epr-20931.13", rom("epr-20923.13", 0x80000, "1404a9c79cd2bae13f60e5e008307417324c3666", Role::Program)),
        ],
        // The protection answers sit 0x10 lower in this program (the two 16-byte replies, found
        // by their bytes); the idle loop too.
        protection_rom: &[(0, 0xbc7e0), (1, 0xbc7f0)],
        idle_loops: Some(&[crate::IdleLoop { start: 0x71184, end: 0x71198 }]),
        ..Variant::BASE
    },
    // Dead or Alive on the Model 2A board (the 2A clone of the 2B set): its program pair
    // (`e19310aa.12/e19311aa.13`) and the six `opr-*` geometry programs every 2A set has; the
    // data, polygons, textures and sound chips are the parent's. Its protection reply is at
    // another ROM address (*confirmed*: docs/games/doa.md, "`doaa`").
    Variant {
        id: "doaa",
        parent: "doa",
        label: "Model 2A",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap("epr-19379b.15", rom("e19310aa.12", 0x80000, "b3e14103570e5f45aed16e1c158e469bc85002ae", Role::Program)),
            swap("epr-19380b.16", rom("e19311aa.13", 0x80000, "dcc2df8e28e1a107867f74248e6ffcac83afe7c0", Role::Program)),
        ],
        add: &[
            rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
            rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
            rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
            rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
            rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
            rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        ],
        board: Some(Board::Model2A),
        protection_rom: &[(0, 0x2830)],
        // The frame wait of the parent's, 0x20 higher (found by its instructions in the program).
        idle_loops: Some(&[crate::IdleLoop { start: 0x2800, end: 0x2814 }]),
        ..Variant::BASE
    },
    // Zero Gunner on the Model 2A board: its program pair (`.12/.13`) and the six 2A `opr-*`
    // geometry programs; the data, polygon, texture and sound chips are the parent's.
    Variant {
        id: "zeroguna",
        parent: "zerogun",
        label: "Model 2A",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap("epr-20439.15", rom("epr-20437.12", 0x80000, "5c6222e07594b4be59b5095f7cc0a164d5895306", Role::Program)),
            swap("epr-20440.16", rom("epr-20438.13", 0x80000, "4672ebdd7d9ccab5e107fda9d322b70583246c7a", Role::Program)),
        ],
        add: &[
            rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
            rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
            rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
            rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
            rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
            rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x548f80, end: 0x548f94 }]),
        board: Some(Board::Model2A),
        ..Variant::BASE
    },
    Variant {
        id: "zerogunaj",
        parent: "zerogun",
        label: "Japan, Model 2A",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap("epr-20439.15", rom("epr-20288.12", 0x80000, "c0d67fbb8f89daacd32bbc1ad0d55a73b60016d8", Role::Program)),
            swap("epr-20440.16", rom("epr-20289.13", 0x80000, "e4c66c6bc9d5433b76ea12cf625fc359439144bb", Role::Program)),
        ],
        add: &[
            rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
            rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
            rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
            rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
            rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
            rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        ],
        idle_loops: Some(&[crate::IdleLoop { start: 0x548fd0, end: 0x548fe4 }]),
        board: Some(Board::Model2A),
        ..Variant::BASE
    },
    // The 2B and 2C clones of Dynamite Cop: one program pair each, the 2A `opr-*` chips and
    // second program pair dropped, the protection answers at their own ROM addresses. They boot,
    // show their title and reach a stage but draw no 3D world (every object gets the same
    // matrix from the copro; cause unknown), so `plays: false` (docs/variants.md, Class C).
    Variant {
        id: "dynmcopb",
        parent: "dynamcop",
        label: "Model 2B",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap("epr-20930.12", rom("progrom0.15", 0x80000, "b81d1ee7203b2f5fb6db4ff4185f4071e99aaedf", Role::Program)),
            swap("epr-20931.13", rom("progrom1.16", 0x80000, "1a45296a5554923cb52b38586e40ceda2517f1bf", Role::Program)),
        ],
        protection_rom: &[(0, 0xc9220), (1, 0xc9230)],
        drop: &["epr-20932.14", "epr-20933.15", "opr-14742a.45", "opr-14743a.46", "opr-14744.58", "opr-14745.59", "opr-14746.62", "opr-14747.63"],
        board: Some(Board::Model2B),
        plays: false,
        ..Variant::BASE
    },
    Variant {
        id: "dyndek2b",
        parent: "dynamcop",
        label: "Dynamite Deka 2, Model 2B",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        title: Some("Dynamite Deka 2"),
        swaps: &[
            swap("epr-20930.12", rom("epr-20936.15", 0x80000, "0a106125ed4a2569b54924130ca2ffa05acf2322", Role::Program)),
            swap("epr-20931.13", rom("epr-20937.16", 0x80000, "ebdd21f269fd8a0798306e349d2985eead7e989f", Role::Program)),
        ],
        protection_rom: &[(0, 0xc9210), (1, 0xc9220)],
        drop: &["epr-20932.14", "epr-20933.15", "opr-14742a.45", "opr-14743a.46", "opr-14744.58", "opr-14745.59", "opr-14746.62", "opr-14747.63"],
        board: Some(Board::Model2B),
        plays: false,
        ..Variant::BASE
    },
    Variant {
        id: "dynmcopc",
        parent: "dynamcop",
        label: "Model 2C",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap("epr-20930.12", rom("epr-20952.15", 0x80000, "85deb1dc1348730a0c9b6ce3679582e7894ff2ed", Role::Program)),
            swap("epr-20931.13", rom("epr-20953.16", 0x80000, "9bea99c043775c00742c20e2f917d211dca09cc5", Role::Program)),
        ],
        protection_rom: &[(0, 0xcd1c0), (1, 0xcd1d0)],
        drop: &["epr-20932.14", "epr-20933.15", "opr-14742a.45", "opr-14743a.46", "opr-14744.58", "opr-14745.59", "opr-14746.62", "opr-14747.63"],
        board: Some(Board::Model2C),
        plays: false,
        ..Variant::BASE
    },
    // Pilot Kids on the Model 2A board: its program pair (`epr-21281.pr0/21282.pr1`) and the six
    // 2A `opr-*` geometry programs; the other chips are the parent's.
    Variant {
        id: "pltkidsa",
        parent: "pltkids",
        label: "Model 2A",
        kind: VariantKind::CrossBoard,
        kind_confirmed: true,
        swaps: &[
            swap("epr-21285a.15", rom("epr-21281.pr0", 0x80000, "5a6295e543d7e68387de0ca4d88e930a0d8ed25c", Role::Program)),
            swap("epr-21286a.16", rom("epr-21282.pr1", 0x80000, "15f3fab6ac2dd40f32bda55503378ab14f998707", Role::Program)),
        ],
        add: &[
            rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
            rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
            rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
            rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
            rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
            rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        ],
        board: Some(Board::Model2A),
        ..Variant::BASE
    },
    // Daytona USA "GTX 2004" and "To The Maxx": hacks (*unconfirmed*: the dat's names and chip
    // names), a program pair each. Neither plays: `daytonam` waits for a device at 0x240000,
    // `daytonagtx` halts, and what `bank0.bin`/`bank1.bin` and `maxx.14` are is unknown
    // (docs/variants.md, Class C), so `plays: false`.
    Variant {
        id: "daytonagtx",
        parent: "daytona",
        label: "GTX 2004",
        kind: VariantKind::Hack,
        kind_confirmed: false,
        swaps: &[
            swap("epr-16722a.12", rom("gtx.12", 0x20000, "643110a3ea5fb6092c469b6b49a396084e985a7a", Role::Program)),
            swap("epr-16723a.13", rom("gtx.13", 0x20000, "ad635540d64e05c7246c9de6439a4e3b3d1cdf08", Role::Program)),
        ],
        plays: false,
        ..Variant::BASE
    },
    Variant {
        id: "daytonam",
        parent: "daytona",
        label: "To The Maxx",
        kind: VariantKind::Hack,
        kind_confirmed: false,
        swaps: &[
            swap("epr-16722a.12", rom("maxx.12", 0x20000, "b1d5f0d41bea2e74fb9346da35a5041f4464265e", Role::Program)),
            swap("epr-16723a.13", rom("maxx.13", 0x20000, "5bc150a77f20a29f54acdf5043fb1e8e55f6b08b", Role::Program)),
        ],
        plays: false,
        ..Variant::BASE
    },
];
