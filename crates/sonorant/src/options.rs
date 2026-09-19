//! Command-line options.

use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: sonorant [options]
       sonorant capture [--app <name or pid>] [--wav <file>] [--seconds <s>]
       sonorant apps

Options:
  --app <name or pid>     capture one app instead of the whole system
  --wav <file>            play a WAV file into the analysis, looped, instead of capturing
  --present-mode <mode>   fifo (default, vsync), mailbox, immediate
  --frame-latency <n>     frames the GPU may queue ahead, 1 to 3 (default 2)
  --backend <name>        vulkan, dx12, gl or metal (default: the platform's best)
  --fullscreen            start fullscreen
  --settings <dir>        keep settings in this folder instead of the usual one
  --pacing-seconds <s>    measure frame pacing for s seconds, print a summary, exit
  --pacing-log <file>     write every frame interval to a CSV file on exit
  --screenshot <file>     save the window's picture as a PNG after a few seconds, exit
  --screenshot-seconds <s>  how long to wait before the screenshot (default 5)
  -h, --help              show this help
  -V, --version           show the version";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub app: Option<String>,
    pub wav: Option<PathBuf>,
    pub present_mode: Option<wgpu::PresentMode>,
    pub frame_latency: u32,
    pub backends: Option<wgpu::Backends>,
    pub fullscreen: bool,
    pub settings_dir: Option<PathBuf>,
    pub pacing_seconds: Option<f64>,
    pub pacing_log: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub screenshot_seconds: f64,
    pub help: bool,
    pub version: bool,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
        let mut o = Options {
            frame_latency: 2,
            screenshot_seconds: 5.0,
            ..Options::default()
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--app" => o.app = Some(value("--app")?),
                "--wav" => o.wav = Some(PathBuf::from(value("--wav")?)),
                "--present-mode" => {
                    o.present_mode = Some(match value("--present-mode")?.as_str() {
                        "fifo" => wgpu::PresentMode::Fifo,
                        "mailbox" => wgpu::PresentMode::Mailbox,
                        "immediate" => wgpu::PresentMode::Immediate,
                        other => return Err(format!("unknown present mode {other}")),
                    })
                }
                "--frame-latency" => {
                    let v = value("--frame-latency")?;
                    o.frame_latency = match v.parse::<u32>() {
                        Ok(n @ 1..=3) => n,
                        _ => return Err(format!("--frame-latency takes 1, 2 or 3, not {v}")),
                    };
                }
                "--backend" => {
                    o.backends = Some(match value("--backend")?.as_str() {
                        "vulkan" => wgpu::Backends::VULKAN,
                        "dx12" => wgpu::Backends::DX12,
                        "gl" => wgpu::Backends::GL,
                        "metal" => wgpu::Backends::METAL,
                        other => return Err(format!("unknown backend {other}")),
                    })
                }
                "--fullscreen" => o.fullscreen = true,
                "--settings" => o.settings_dir = Some(PathBuf::from(value("--settings")?)),
                "--pacing-seconds" => {
                    let v = value("--pacing-seconds")?;
                    o.pacing_seconds = match v.parse::<f64>() {
                        Ok(s) if s > 0.0 => Some(s),
                        _ => {
                            return Err(format!(
                                "--pacing-seconds takes a positive number, not {v}"
                            ));
                        }
                    };
                }
                "--pacing-log" => o.pacing_log = Some(PathBuf::from(value("--pacing-log")?)),
                "--screenshot" => o.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--screenshot-seconds" => {
                    let v = value("--screenshot-seconds")?;
                    o.screenshot_seconds = match v.parse::<f64>() {
                        Ok(s) if s >= 0.0 => s,
                        _ => {
                            return Err(format!(
                                "--screenshot-seconds takes a number of seconds, not {v}"
                            ));
                        }
                    };
                }
                "-h" | "--help" => o.help = true,
                "-V" | "--version" => o.version = true,
                other => return Err(format!("unknown option {other}")),
            }
        }
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let o = parse(&[]).unwrap();
        assert_eq!(o.frame_latency, 2);
        assert_eq!(o.present_mode, None);
        assert!(!o.fullscreen);
    }

    #[test]
    fn values() {
        let o = parse(&[
            "--present-mode",
            "mailbox",
            "--pacing-seconds",
            "30",
            "--backend",
            "dx12",
        ])
        .unwrap();
        assert_eq!(o.present_mode, Some(wgpu::PresentMode::Mailbox));
        assert_eq!(o.pacing_seconds, Some(30.0));
        assert_eq!(o.backends, Some(wgpu::Backends::DX12));
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(
            parse(&["--frame-latency", "9"])
                .unwrap_err()
                .contains("1, 2 or 3")
        );
        assert!(
            parse(&["--pacing-seconds"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(parse(&["--wat"]).unwrap_err().contains("unknown option"));
    }
}
