use crate::{Board, Game, Role, RomFile};

/// Model 2C-CRX. Roles and interleave order follow the board wiring;
/// pairs are listed low half-word first (note textures: `.27` before `.25`).
pub static TOPSKATR: Game = Game {
    id: "topskatr",
    title: "Top Skater",
    board: Board::Model2C,
    files: &[
        RomFile {
            name: "epr-19755a.15",
            size: 0x80000,
            sha1: "5396da414beeb918e6f38f25a43dd76345a0c8ed",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19756a.16",
            size: 0x80000,
            sha1: "06d0f609257ba476e6bd3b956e0850e7167429ce",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19735.11",
            size: 0x400000,
            sha1: "49afc91467f08befaf34e743cbe823de3e3c9d85",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19736.12",
            size: 0x400000,
            sha1: "de2c739f71e51166263446b9f6a566866ab8bee8",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19737.9",
            size: 0x400000,
            sha1: "71d5ba434328a81969bfdc71ac1160c5ff3ae9d3",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19738.10",
            size: 0x400000,
            sha1: "68c9db242ef7e8f98979e968a09e4b093bc5d470",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19740.27",
            size: 0x400000,
            sha1: "c90fa3b42d87291ea459ccc137f3a2f3eb7efec0",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19739.25",
            size: 0x400000,
            sha1: "a82744bff5dcdfae296c7c3e8c3fbfda26324e85",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19741.17",
            size: 0x200000,
            sha1: "8664059f157626e4bbdcf8357e3d30b37d3c25b8",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19742.21",
            size: 0x200000,
            sha1: "3e68aec090f36a60b3b70bc90f09e2f9ce088718",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19743.29",
            size: 0x200000,
            sha1: "a5f6b24e6526d0d2ef9c526c273c018d1e0fed59",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19744.30",
            size: 0x200000,
            sha1: "4952b764e6bf6cd735018738c5eff08781ee2315",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19745.32",
            size: 0x400000,
            sha1: "415f9d0793a697cb1719bbd96370f4a741866527",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19746.34",
            size: 0x400000,
            sha1: "ca76f211d68b6b55678a4d7949bfd2ddef1b1710",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19747.18s",
            size: 0x400000,
            sha1: "4c67c1e1d58a3034bbd711252a78689db9f235bb",
            role: Role::MpegData,
        },
        RomFile {
            name: "mpr-19748.20s",
            size: 0x400000,
            sha1: "fd4da4cf40c4342c6263cf22eee5968292a4d2c0",
            role: Role::MpegData,
        },
        RomFile {
            name: "mpr-19749.22s",
            size: 0x400000,
            sha1: "6ee6b2eb2ea400bdb9c0a9b4a126b4b86886e813",
            role: Role::MpegData,
        },
        RomFile {
            name: "mpr-19750.24s",
            size: 0x400000,
            sha1: "40e2a2980c89049c339fefd48bf7aac79962cd2e",
            role: Role::MpegData,
        },
        RomFile {
            name: "mpr-19759.31",
            size: 0x80000,
            sha1: "7b205085965d6694f8e75e29c4028f7cb6f631ab",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-19760.2s",
            size: 0x20000,
            sha1: "a302209bfe0f1491dff2da64b32cfaa13c3d3304",
            role: Role::MpegCpu,
        },
    ],
};
