//! KiCad's PCB editor running with its IPC API, for the end-to-end test of
//! routing the board open in KiCad: KiCad 9 and 10 serve the API only from the
//! GUI, so the editor runs from the Docker image on a virtual display, with
//! traxide's plugin installed, whose toolbar buttons the test presses.
//!
//! Linux only: the API's Unix socket is shared through a bind mount, which
//! Docker Desktop does not pass through on macOS or Windows. Pressing the
//! button takes ImageMagick's `import` and `xdotool` on the host.

use crate::kicad_package::IDENTIFIER;
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use traxide_board_access::testing;
use traxide_contracts::Design;

/// How long KiCad may take to start and load the board.
const STARTUP: Duration = Duration::from_secs(90);
/// How long the plugin may take to route a test board.
const ROUTING: Duration = Duration::from_secs(120);

/// The buttons the plugin adds to the editor's toolbar.
#[derive(Debug, Clone, Copy)]
pub enum Button {
    RouteBoard,
    RouteSelectedNets,
}

impl Button {
    /// The colour the button's light icon is filled with, by which the test
    /// finds it on the screen (`kicad-plugin/plugins/icon*.png`).
    fn icon_fill(self) -> [u8; 3] {
        match self {
            Self::RouteBoard => [27, 127, 90],
            Self::RouteSelectedNets => [27, 90, 127],
        }
    }

    /// The name of the process the button starts, as `pgrep -x` matches it:
    /// Linux keeps only the first 15 characters of a process's name.
    fn process(self) -> &'static str {
        match self {
            Self::RouteBoard => "traxide",
            Self::RouteSelectedNets => "traxide-selecte",
        }
    }
}

/// A PCB editor with a board open; stopped when dropped.
pub struct RunningKicad {
    container: String,
    display: Child,
    /// Where KiCad created its API's socket.
    pub socket: PathBuf,
    /// The board's folder, shared with KiCad, which saves the board there.
    pub project: PathBuf,
}

impl RunningKicad {
    /// Starts KiCad from `image` on a copy of the board `board` and its project
    /// files, all placed in `folder`, with the plugin package `package`
    /// installed, and waits until it serves the board.
    pub fn open(image: &str, board: &Path, folder: &Path, package: &Path) -> Result<Self> {
        let config = folder.join("config");
        let share = folder.join("share");
        let socket_folder = folder.join("socket");
        let project = folder.join("project");
        if folder.exists() {
            std::fs::remove_dir_all(folder)?;
        }
        for created in [&config, &share, &socket_folder, &project] {
            std::fs::create_dir_all(created)?;
        }
        let stem = board.file_stem().context("a board file")?;
        for extension in ["kicad_pcb", "kicad_pro", "kicad_dru"] {
            let file = board.with_extension(extension);
            if file.is_file() {
                std::fs::copy(&file, project.join(stem).with_extension(extension))?;
            }
        }
        // The image runs KiCad as its own user, who must write the
        // configuration, the socket and the saved board.
        run(Command::new("chmod").arg("-R").arg("a+rwX").arg(folder))?;
        write_configuration(image, &config)?;
        install_plugin(package, &config, &share)?;

        let container = format!("traxide-e2e-kicad-{}", std::process::id());
        let board_name = board.file_name().context("a board file")?.to_string_lossy();
        // xvfb-run keeps a virtual display for as long as the editor runs; -ac
        // because the container's host name does not match its cookie, and the
        // throwaway display has nothing to protect.
        let display = Command::new("xvfb-run")
            .args(["-a", "-s", "-ac -screen 0 1600x1000x24"])
            .args([
                "docker", "run", "--rm", "--name", &container, "-e", "DISPLAY",
            ])
            .args(["-v", "/tmp/.X11-unix:/tmp/.X11-unix"])
            .arg("-v")
            .arg(format!("{}:/home/kicad/.config/kicad", config.display()))
            .arg("-v")
            .arg(format!(
                "{}:/home/kicad/.local/share/kicad",
                share.display()
            ))
            .arg("-v")
            .arg(format!("{}:/tmp/kicad", socket_folder.display()))
            .arg("-v")
            .arg(format!("{}:/project", project.display()))
            // umask 000: the image's user creates the API's socket, which the
            // host's user, who may be another, must be able to connect to.
            .args([
                image,
                "sh",
                "-c",
                &format!("umask 000 && exec pcbnew '/project/{board_name}'"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting KiCad on a virtual display: are xvfb-run and docker installed?")?;
        let kicad = Self {
            container,
            display,
            socket: socket_folder.join("api.sock"),
            project,
        };
        kicad.wait_for_the_board()?;
        Ok(kicad)
    }

    /// KiCad creates its socket before it has loaded the board, and answers
    /// that no board is open until then.
    fn wait_for_the_board(&self) -> Result<()> {
        let started = Instant::now();
        loop {
            match self.design() {
                Ok(_) => return Ok(()),
                Err(error) if started.elapsed() > STARTUP => {
                    bail!(
                        "KiCad did not serve the board within {} s: {error:#}",
                        STARTUP.as_secs()
                    )
                }
                Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        }
    }

    /// The design of the open board, read through KiCad's API.
    pub fn design(&self) -> Result<Design> {
        Ok(testing::load_design_in_kicad(&self.socket, &self.project)?)
    }

    pub fn save(&self) -> Result<()> {
        Ok(testing::save_board_in_kicad(&self.socket)?)
    }

    pub fn undo(&self) -> Result<()> {
        Ok(testing::undo_in_kicad(&self.socket)?)
    }

    /// Selects a footprint with a pad of a net, as clicking it does, and names
    /// it by its reference.
    pub fn select_a_footprint(&self) -> Result<String> {
        Ok(testing::select_a_footprint_in_kicad(&self.socket)?)
    }

    /// Presses the plugin's `button`, as a designer would, and waits until the
    /// plugin has changed the board `before` and finished.
    pub fn route_with_the_plugin(&self, button: Button, before: &Design) -> Result<()> {
        let display = self.display()?;
        let (x, y) = self.find_the_button(&display, button)?;
        run(Command::new("xdotool")
            .env("DISPLAY", &display)
            .env_remove("XAUTHORITY")
            .args(["mousemove", &x.to_string(), &y.to_string(), "click", "1"]))?;
        let started = Instant::now();
        loop {
            let plugin_running = Command::new("docker")
                .args(["exec", &self.container, "pgrep", "-x", button.process()])
                .output()?
                .status
                .success();
            if !plugin_running && self.design()? != *before {
                return Ok(());
            }
            ensure!(
                started.elapsed() < ROUTING,
                "the plugin did not route the board within {} s of pressing its button",
                ROUTING.as_secs()
            );
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// The X display the editor shows on, which xvfb-run picked.
    fn display(&self) -> Result<String> {
        let output = Command::new("docker")
            .args(["exec", &self.container, "printenv", "DISPLAY"])
            .output()?;
        ensure!(output.status.success(), "the editor has no display");
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// The middle of the plugin's button, found on a screenshot by the colour
    /// of its icon; KiCad adds plugin buttons to the toolbar a little after
    /// loading the board.
    fn find_the_button(&self, display: &str, button: Button) -> Result<(usize, usize)> {
        let started = Instant::now();
        loop {
            let screenshot = Command::new("import")
                .env_remove("XAUTHORITY")
                .args([
                    "-display", display, "-window", "root", "-depth", "8", "ppm:-",
                ])
                .output()
                .context("taking a screenshot: is ImageMagick installed?")?;
            ensure!(screenshot.status.success(), "taking a screenshot failed");
            if let Some(middle) = middle_of_colour(&screenshot.stdout, button.icon_fill())? {
                return Ok(middle);
            }
            ensure!(
                started.elapsed() < STARTUP,
                "the editor shows no traxide button: KiCad did not load the plugin"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

/// Installs the plugin package as KiCad's *Install from File* does, for each
/// KiCad version the configuration in `config` is for: `plugins/` and
/// `resources/` go into folders named after the identifier in `3rdparty`.
fn install_plugin(package: &Path, config: &Path, share: &Path) -> Result<()> {
    let folder = IDENTIFIER.replace('.', "_");
    let unpacked = share.join("unpacked");
    // unzip, like KiCad on Linux, keeps the executable bit.
    run(Command::new("unzip")
        .args(["-q", "-o"])
        .arg(package)
        .arg("-d")
        .arg(&unpacked))?;
    for version in std::fs::read_dir(config)? {
        let version = version?;
        if !version.file_type()?.is_dir() {
            continue;
        }
        let third_party = share.join(version.file_name()).join("3rdparty");
        for part in ["plugins", "resources"] {
            let installed = third_party.join(part).join(&folder);
            std::fs::create_dir_all(installed.parent().context("a parent")?)?;
            run(Command::new("cp")
                .arg("-a")
                .arg(unpacked.join(part))
                .arg(&installed))?;
        }
    }
    std::fs::remove_dir_all(&unpacked)?;
    // KiCad, as the image's user, writes beside the plugin.
    run(Command::new("chmod").arg("-R").arg("a+rwX").arg(share))
}

/// The middle of the pixels of `colour` in a binary PPM image (P6, 8 bits),
/// if it has enough of them to be the icon.
fn middle_of_colour(ppm: &[u8], colour: [u8; 3]) -> Result<Option<(usize, usize)>> {
    let mut header = Vec::new();
    let mut at = 0;
    while header.len() < 4 {
        let start = at;
        while at < ppm.len() && !ppm[at].is_ascii_whitespace() {
            at += 1;
        }
        header.push(std::str::from_utf8(&ppm[start..at])?);
        at += 1;
    }
    ensure!(
        header[0] == "P6" && header[3] == "255",
        "not an 8-bit binary PPM"
    );
    let width: usize = header[1].parse()?;
    let pixels = ppm.get(at..).context("pixels")?;
    let (mut count, mut sum_x, mut sum_y) = (0, 0, 0);
    for (index, pixel) in pixels.as_chunks::<3>().0.iter().enumerate() {
        if *pixel == colour {
            count += 1;
            sum_x += index % width;
            sum_y += index / width;
        }
    }
    // A 24-pixel icon is mostly its fill colour.
    Ok((count >= 100).then(|| (sum_x / count, sum_y / count)))
}

impl Drop for RunningKicad {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container])
            .output();
        let _ = self.display.wait();
    }
}

/// KiCad greets a new user with a wizard, a modal dialog that keeps the API
/// busy until someone closes it, unless all three global library tables exist
/// and the wizard's two questions count as answered.
///
/// The image's own configuration holds two of the tables, in a folder named
/// after its KiCad version. Its user copies them, and the files written here
/// beside them, and opens them all to everyone: KiCad rewrites its settings,
/// and only the owner of a file may change its mode, while the host's user
/// may be another.
fn write_configuration(image: &str, config: &Path) -> Result<()> {
    let seed = config.with_file_name("seed");
    std::fs::create_dir_all(&seed)?;
    std::fs::write(
        seed.join("design-block-lib-table"),
        "(design_block_lib_table\n\t(version 7)\n\t(lib (name \"KiCad\") (type \"Table\") \
         (uri \"/usr/share/kicad/template/design-block-lib-table\") (options \"\") \
         (descr \"KiCad Default Libraries\"))\n)\n",
    )?;
    std::fs::write(
        seed.join("kicad_common.json"),
        r#"{
  "api": { "enable_server": true },
  "do_not_show_again": { "data_collection_prompt": true, "update_check_prompt": true }
}
"#,
    )?;
    run(Command::new("docker")
        .args(["run", "--rm", "-v"])
        .arg(format!("{}:/out", config.display()))
        .arg("-v")
        .arg(format!("{}:/seed:ro", seed.display()))
        .args([
            image,
            "sh",
            "-c",
            "cp -r ~/.config/kicad/. /out/ && for version in /out/*/; do cp /seed/* \"$version\"; done \
             && chmod -R a+rwX /out/*",
        ]))?;
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("running {command:?}"))?;
    ensure!(
        output.status.success(),
        "{command:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppm(width: usize, height: usize, paint: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut image = format!("P6\n{width} {height}\n255\n").into_bytes();
        for y in 0..height {
            for x in 0..width {
                image.extend(paint(x, y));
            }
        }
        image
    }

    #[test]
    fn finds_the_middle_of_the_icon_on_a_screenshot() {
        let screenshot = ppm(100, 50, |x, y| {
            if (60..80).contains(&x) && (10..30).contains(&y) {
                Button::RouteBoard.icon_fill()
            } else {
                [240, 240, 240]
            }
        });
        assert_eq!(
            middle_of_colour(&screenshot, Button::RouteBoard.icon_fill()).expect("a PPM"),
            Some((69, 19))
        );
    }

    #[test]
    fn finds_no_icon_on_a_screenshot_without_it() {
        let screenshot = ppm(100, 50, |_, _| [240, 240, 240]);
        assert_eq!(
            middle_of_colour(&screenshot, Button::RouteBoard.icon_fill()).expect("a PPM"),
            None
        );
    }
}
