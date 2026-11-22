use super::*;

fn bindings(id: &str) -> Bindings {
    Bindings::defaults(&romset::games::playable(id).unwrap().controls)
}

fn slot(b: &Bindings, name: &str) -> Slot {
    b.slot_named(name).unwrap_or_else(|| panic!("{name}"))
}

#[test]
fn only_wheel_and_gun_games_have_cabinet_art() {
    let art = |id| cabinet_art(Some(romset::games::playable(id).unwrap()));
    assert_eq!(art("hotd"), Some(Art::Gun));
    assert_eq!(art("daytona"), Some(Art::Wheel));
    assert_eq!(art("topskatr"), None);
    assert_eq!(cabinet_art(None), None);
}

#[test]
fn rows_follow_the_device() {
    let b = bindings("topskatr");
    let (keys, hidden) = rows(&b, Device::Keys, PadStyle::Offset, false);
    assert!(hidden > 0, "the unbound inputs are folded away");
    assert!(keys.iter().all(|r| b.accepts(r.slot) == Accepts::Inputs), "no axis rows on the keyboard");
    let jump = keys.iter().find(|r| r.slot == slot(&b, "jumpfront")).unwrap();
    assert_eq!(jump.caps.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Z", "↑"]);
    let (pad, _) = rows(&b, Device::Pad, PadStyle::Offset, false);
    let jump = pad.iter().find(|r| r.slot == slot(&b, "jumpfront")).unwrap();
    assert_eq!(jump.caps.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["A"]);
    let (side, _) = rows(&b, Device::Pad, PadStyle::SideBySide, false);
    assert_eq!(side.iter().find(|r| r.slot == slot(&b, "jumpfront")).unwrap().caps[0].text, "Cross");
    let (all, hidden_none) = rows(&b, Device::Pad, PadStyle::Offset, true);
    assert!(all.len() > pad.len() && hidden_none == 0);
}

#[test]
fn clear_and_restore_one_device() {
    let mut b = bindings("topskatr");
    let jump = slot(&b, "jumpfront");
    assert!(clear_device(&mut b, jump, Device::Keys));
    assert_eq!(b.get(jump).unwrap().inputs(), [Input::Pad(PadButton::South)]);
    assert!(!clear_device(&mut b, jump, Device::Keys), "nothing left to clear there");
    assert!(!b.is_default(jump));
    b.reset(jump);
    assert!(b.is_default(jump));
    // An axis goes to none.
    let axis = b.slots().iter().map(|s| s.0).find(|s| b.accepts(*s) == Accepts::Axis).unwrap();
    assert!(clear_device(&mut b, axis, Device::Pad));
    assert_eq!(b.get(axis).unwrap().axis().axis, PadAxis::None);
}

#[test]
fn a_doubled_key_is_marked_and_reported() {
    let mut b = bindings("topskatr");
    let (jump, tail) = (slot(&b, "jumpfront"), slot(&b, "jumptail"));
    assert_eq!(conflict_text(&b, jump), None);
    b.assign(tail, None, Input::Key("KeyZ".into()), romset::bindings::RESERVED_KEYS).unwrap();
    let text = conflict_text(&b, tail).unwrap();
    assert!(text.starts_with("Z is also ") && text.contains(&b.label(jump)), "{text}");
    let (list, _) = rows(&b, Device::Keys, PadStyle::Offset, false);
    let r = list.iter().find(|r| r.slot == tail).unwrap();
    assert!(r.caps.iter().any(|c| c.text == "Z" && c.clash));
    assert!(r.also.as_deref().is_some_and(|a| a.starts_with("also ")));
    assert!(!list.iter().find(|r| r.slot == slot(&b, "start")).unwrap().caps.iter().any(|c| c.clash));
}

#[test]
fn parts_follow_the_bindings() {
    let b = bindings("topskatr");
    let (list, _) = rows(&b, Device::Pad, PadStyle::Offset, false);
    let (on, loose) = part_actions(&b, Device::Pad, Art::PadOffset, &list);
    assert!(loose.is_empty());
    assert!(on.iter().any(|o| o.part == Part::Pad(PadButton::South) && o.slot == Some(slot(&b, "jumpfront"))));
    assert!(on.iter().any(|o| o.part == Part::Pad(PadButton::LeftStick)));
    // A wheel game: the wheel, the pedals, the paddles.
    let b = bindings("daytona");
    let (list, _) = rows(&b, Device::Cabinet, PadStyle::Offset, false);
    let (on, loose) = part_actions(&b, Device::Cabinet, Art::Wheel, &list);
    for part in [WheelPart::Rim, WheelPart::Gas, WheelPart::Brake, WheelPart::PaddleRight, WheelPart::PaddleLeft] {
        assert!(on.iter().any(|o| o.part == Part::Wheel(part)), "{part:?}");
    }
    assert!(loose.contains(&"Coin".to_string()), "{loose:?}");
    // A gun game: the trigger, and where it points.
    let b = bindings("hotd");
    let (list, _) = rows(&b, Device::Cabinet, PadStyle::Offset, false);
    let (on, _) = part_actions(&b, Device::Cabinet, Art::Gun, &list);
    assert!(on.iter().any(|o| o.part == Part::Gun(GunPart::Trigger)));
    assert!(on.iter().any(|o| o.part == Part::Gun(GunPart::Muzzle)));
}

#[test]
fn tuning_blocks_follow_the_axes() {
    assert!(blocks(&bindings("vf2")).is_empty());
    let b = bindings("daytona");
    let k = blocks(&b);
    let kinds: Vec<Tuned> = k.iter().map(|k| k.kind).collect();
    // The wheel steers; the pedals are triggers.
    assert_eq!(kinds, [Tuned::Steer, Tuned::Pedal], "{k:?}");
    // A gun game's sticks aim: the stick tuning, no steering.
    assert!(blocks(&bindings("hotd")).iter().all(|k| k.kind != Tuned::Steer));
}

/// Every game's steering inputs are bidirectional analogs of a vehicle.
#[test]
fn steering_inputs_are_marked() {
    let steering: Vec<&str> =
        romset::games::ALL.iter().filter(|g| g.controls.analogs.iter().any(|a| a.steering)).map(|g| g.id).collect();
    for id in ["daytona", "srallyc", "indy500", "stcc", "manxtt", "motoraid", "waverunr", "topskatr", "desert"] {
        assert!(steering.contains(&id), "{id}");
    }
    for g in romset::games::ALL {
        for a in g.controls.analogs.iter().filter(|a| a.steering) {
            assert_ne!(a.centre, 0, "{}: {}", g.id, a.name);
        }
    }
    assert!(!steering.contains(&"rchase2") && !steering.contains(&"skytargt"));
}

#[test]
fn tweaks_stay_in_range_and_snap() {
    let mut r = Response::STICK;
    tune(&mut r, 0, Tweak::By(-100));
    assert_eq!(r.dead, 0);
    tune(&mut r, 0, Tweak::By(100));
    assert_eq!(r.dead, Response::MAX_DEAD);
    tune(&mut r, 0, Tweak::To(0.5));
    assert_eq!(r.dead, 20);
    tune(&mut r, 1, Tweak::By(1));
    assert_eq!(r.sens, 105);
    tune(&mut r, 1, Tweak::To(0.0));
    assert_eq!(r.sens, 50);
    tune(&mut r, 1, Tweak::To(2.0));
    assert_eq!(r.sens, 150);
    tune(&mut r, 2, Tweak::Cycle);
    assert_eq!(r.curve, Curve::Fine);
    tune(&mut r, 2, Tweak::By(5));
    assert_eq!(r.curve, Curve::Quick);
    tune(&mut r, 2, Tweak::Cycle);
    assert_eq!(r.curve, Curve::Linear);
}

#[test]
fn smoothing_steps_and_cycles() {
    use crate::steer::Smoothing;
    let mut m = Smoothing::default();
    assert_eq!(m, Smoothing::Light);
    tune_smoothing(&mut m, Tweak::By(-5));
    assert_eq!(m, Smoothing::Off);
    tune_smoothing(&mut m, Tweak::By(5));
    assert_eq!(m, Smoothing::Strong);
    tune_smoothing(&mut m, Tweak::Cycle);
    assert_eq!(m, Smoothing::Off);
}
