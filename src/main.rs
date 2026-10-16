use quadcast_s_mac::effects::{frames, Effect, EffectSpec, Speed};
use quadcast_s_mac::protocol::{self, Frame, Rgb};
use std::process::exit;

const HELP: &str = "\
quadcast - control a HyperX QuadCast S on macOS

LIGHTING (stored on the mic, keeps running with this app closed)
  quadcast light solid <colour> [bottom-colour] [--brightness 0-100]
  quadcast light effect <solid|blink|breathing|cycle|wave|lightning>
                [--colour C] [--colour2 C] [--speed slow|normal|fast] [--brightness 0-100]
  quadcast light off
  Add --live [seconds] to show it without storing it (stops when the app exits).
  Colours: red green blue white yellow cyan magenta orange purple, or hex like ff8800.

AUDIO
  quadcast mic level [0-100]              show or set mic level
  quadcast mic mute [on|off|toggle]
  quadcast headphones level [0-100]       volume of the mic's headphone jack
  quadcast headphones mute [on|off|toggle]
  quadcast monitor [on|off]               hear your mic in the headphones
  quadcast monitor level [0-100]

OTHER
  quadcast status                          what was found and current settings
  quadcast audio controls                  list every audio control macOS exposes
  quadcast packets <light args>            print the raw reports (for testing)
";

fn die(msg: &str) -> ! {
    eprintln!("error: {}", msg);
    exit(1)
}

fn colour(s: &str) -> Rgb {
    Rgb::parse(s).unwrap_or_else(|| die(&format!("unknown colour '{}'", s)))
}

fn percent(s: &str) -> u8 {
    match s.parse::<u8>() {
        Ok(v) if v <= 100 => v,
        _ => die(&format!("'{}' is not a number from 0 to 100", s)),
    }
}

/// What a `light` command asks for.
struct LightJob {
    frames: Vec<Frame>,
    live: Option<u64>,
}

fn take_flag(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    if i < args.len() && !args[i].starts_with("--") {
        Some(args.remove(i))
    } else {
        Some(String::new())
    }
}

fn parse_light(mut a: Vec<String>) -> LightJob {
    let brightness = take_flag(&mut a, "--brightness").map(|v| percent(&v)).unwrap_or(100);
    let live = take_flag(&mut a, "--live").map(|v| if v.is_empty() { 0 } else { v.parse().unwrap_or_else(|_| die("--live takes seconds")) });
    let c1 = take_flag(&mut a, "--colour").or_else(|| take_flag(&mut a, "--color")).filter(|s| !s.is_empty());
    let c2 = take_flag(&mut a, "--colour2").or_else(|| take_flag(&mut a, "--color2")).filter(|s| !s.is_empty());
    let speed = take_flag(&mut a, "--speed")
        .map(|v| Speed::parse(&v).unwrap_or_else(|| die("speed is slow, normal or fast")))
        .unwrap_or(Speed::Normal);

    let kind = a.first().map(String::as_str).unwrap_or("");
    let frames = match kind {
        "off" => vec![Frame::both(Rgb::OFF)],
        "solid" => {
            let top = a.get(1).map(|s| colour(s)).unwrap_or_else(|| die("light solid needs a colour"));
            let bottom = a.get(2).map(|s| colour(s));
            frames(&EffectSpec { effect: Effect::Solid, colour: top, colour2: bottom, speed, brightness })
        }
        "effect" => {
            let name = a.get(1).unwrap_or_else(|| die("light effect needs a name"));
            let effect = Effect::parse(name).unwrap_or_else(|| die(&format!("unknown effect '{}'", name)));
            let c = c1.map(|s| colour(&s)).unwrap_or(Rgb(255, 0, 0));
            frames(&EffectSpec { effect, colour: c, colour2: c2.map(|s| colour(&s)), speed, brightness })
        }
        _ => die("light needs solid, effect or off (see quadcast help)"),
    };
    LightJob { frames, live }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn on_off(cur: Option<bool>, arg: &str) -> bool {
    match arg {
        "on" => true,
        "off" => false,
        "toggle" => !cur.unwrap_or(false),
        _ => die("use on, off or toggle"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn")).collect();
    // Opened from Finder / Launchpad: show the window.
    #[cfg(target_os = "macos")]
    if args.is_empty() {
        quadcast_s_mac::mac::gui::run();
        return;
    }
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();

    match cmd {
        "help" | "-h" | "--help" => print!("{}", HELP),
        "packets" => {
            let job = parse_light(rest);
            let reports = if job.live.is_some() {
                job.frames.iter().flat_map(|f| protocol::live_reports(*f)).collect::<Vec<_>>()
            } else {
                protocol::save_reports(&job.frames).unwrap_or_else(|e| die(&e))
            };
            for r in reports {
                println!("{}", protocol::hex(&r));
            }
        }
        _ => run_device(cmd, rest),
    }
}

#[cfg(not(target_os = "macos"))]
fn run_device(_cmd: &str, _rest: Vec<String>) {
    die("hardware commands run on macOS only (try: quadcast packets light solid red)")
}

#[cfg(target_os = "macos")]
fn run_device(cmd: &str, rest: Vec<String>) {
    use quadcast_s_mac::mac::audio::{AudioDevice, Side};
    use quadcast_s_mac::mac::hid::Light;
    use std::time::{Duration, Instant};

    let audio = || AudioDevice::find().unwrap_or_else(|e| die(&e));
    let show = |label: &str, v: Option<String>| println!("{:<20} {}", label, v.unwrap_or_else(|| "not available on macOS".into()));

    match cmd {
        "status" => {
            match Light::open() {
                Ok(l) => println!("{:<20} found ({})", "lighting", l.product),
                Err(e) => println!("{:<20} {}", "lighting", e),
            }
            match AudioDevice::find() {
                Ok(a) => {
                    println!("{:<20} {}", "audio device", a.name);
                    show("mic level", a.volume(Side::Mic).map(|v| format!("{}%", v)));
                    show("mic muted", a.muted(Side::Mic).map(|v| v.to_string()));
                    show("headphone level", a.volume(Side::Headphones).map(|v| format!("{}%", v)));
                    show("headphones muted", a.muted(Side::Headphones).map(|v| v.to_string()));
                    show("monitoring", a.monitoring().map(|v| v.to_string()));
                    show("monitoring level", a.monitoring_level().map(|v| format!("{}%", v)));
                }
                Err(e) => println!("{:<20} {}", "audio device", e),
            }
        }
        "light" => {
            let job = parse_light(rest);
            let light = Light::open().unwrap_or_else(|e| die(&e));
            match job.live {
                None => {
                    let reports = protocol::save_reports(&job.frames).unwrap_or_else(|e| die(&e));
                    light.send_all(&reports).unwrap_or_else(|e| die(&e));
                    println!("stored on the mic ({} frame{})", job.frames.len(), if job.frames.len() == 1 { "" } else { "s" });
                }
                Some(secs) => {
                    let until = (secs > 0).then(|| Instant::now() + Duration::from_secs(secs));
                    println!("showing live{} - press Ctrl+C to stop", until.map(|_| format!(" for {}s", secs)).unwrap_or_default());
                    let mut i = 0usize;
                    loop {
                        if until.is_some_and(|t| Instant::now() >= t) {
                            break;
                        }
                        let f = job.frames[i % job.frames.len()];
                        light.send_all(&protocol::live_reports(f)).unwrap_or_else(|e| die(&e));
                        i += 1;
                    }
                }
            }
        }
        "mic" | "headphones" => {
            let side = if cmd == "mic" { Side::Mic } else { Side::Headphones };
            let a = audio();
            match (rest.first().map(String::as_str), rest.get(1)) {
                (Some("level"), None) => show("level", a.volume(side).map(|v| format!("{}%", v))),
                (Some("level"), Some(v)) => {
                    a.set_volume(side, percent(v)).unwrap_or_else(|e| die(&e));
                    println!("level set to {}%", percent(v));
                }
                (Some("mute"), None) => show("muted", a.muted(side).map(|v| v.to_string())),
                (Some("mute"), Some(v)) => {
                    let on = on_off(a.muted(side), v);
                    a.set_muted(side, on).unwrap_or_else(|e| die(&e));
                    println!("{}", if on { "muted" } else { "unmuted" });
                }
                _ => die("use: level [0-100] or mute [on|off|toggle]"),
            }
        }
        "monitor" => {
            let a = audio();
            match (rest.first().map(String::as_str), rest.get(1)) {
                (None, _) => show("monitoring", a.monitoring().map(|v| v.to_string())),
                (Some("level"), None) => show("monitoring level", a.monitoring_level().map(|v| format!("{}%", v))),
                (Some("level"), Some(v)) => {
                    a.set_monitoring_level(percent(v)).unwrap_or_else(|e| die(&e));
                    println!("monitoring level set to {}%", percent(v));
                }
                (Some(v), _) => {
                    let on = on_off(a.monitoring(), v);
                    a.set_monitoring(on).unwrap_or_else(|e| die(&e));
                    println!("monitoring {}", if on { "on" } else { "off" });
                }
            }
        }
        "audio" if rest.first().map(String::as_str) == Some("controls") => {
            let a = audio();
            println!("{}", a.name);
            for line in a.describe_controls() {
                println!("  {}", line);
            }
        }
        _ => die("unknown command (see quadcast help)"),
    }
}
