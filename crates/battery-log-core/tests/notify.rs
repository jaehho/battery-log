use battery_log_core::notify::{decide, Level, Op, Reading};

fn at(level: Level, percent: f64, tte: i64) -> Reading {
    Reading {
        level,
        percent,
        time_to_empty: tte,
    }
}

#[test]
fn quiet_while_level_holds() {
    assert_eq!(
        decide(Level::Clear, &at(Level::Clear, 80.0, 0)),
        Op::Nothing
    );
    // Percent moving inside a level must not re-pop the notification.
    assert_eq!(decide(Level::Low, &at(Level::Low, 14.0, 900)), Op::Nothing);
}

#[test]
fn escalates_in_place() {
    let Op::Show(low) = decide(Level::Clear, &at(Level::Low, 19.6, 4500)) else {
        panic!()
    };
    assert_eq!(
        (low.urgency, low.body.as_str()),
        (1, "20%, about 1h 15m left")
    );
    let Op::Show(crit) = decide(Level::Low, &at(Level::Critical, 9.0, 0)) else {
        panic!()
    };
    assert_eq!((crit.urgency, crit.body.as_str()), (2, "9%. Plug in soon."));
    let Op::Show(act) = decide(Level::Critical, &at(Level::Action, 5.0, 0)) else {
        panic!()
    };
    assert_eq!(act.summary, "Hibernating");
}

#[test]
fn starting_low_warns_immediately() {
    // A daemon (re)started below a threshold jumps straight to that level.
    assert!(
        matches!(decide(Level::Clear, &at(Level::Critical, 8.0, 0)), Op::Show(n) if n.urgency == 2)
    );
}

#[test]
fn plugging_in_closes() {
    assert_eq!(
        decide(Level::Critical, &at(Level::Clear, 8.0, 0)),
        Op::Close
    );
}

#[test]
fn falling_back_a_level_updates() {
    // Charging from critical past the critical line, still below low.
    assert!(
        matches!(decide(Level::Critical, &at(Level::Low, 11.0, 0)), Op::Show(n) if n.urgency == 1)
    );
}

#[test]
fn upower_levels_map() {
    assert_eq!(Level::from_upower(0), Level::Clear);
    assert_eq!(Level::from_upower(1), Level::Clear);
    assert_eq!(Level::from_upower(2), Level::Clear);
    assert_eq!(Level::from_upower(3), Level::Low);
    assert_eq!(Level::from_upower(4), Level::Critical);
    assert_eq!(Level::from_upower(5), Level::Action);
}
