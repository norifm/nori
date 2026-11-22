# Build your first plugin

In this tutorial you build Balance, a plugin that moves the music to the left or the right with a
slider. You build it, install it in nori's desktop app, hear it, and then update it with a second
setting. It takes about 20 minutes.

You need:

- Rust, installed with rustup. nori's own plugins build with Rust 1.97.1.
- nori's desktop app, with a profile that plays music.
- Headphones or two speakers, to hear the balance move.

For what a plugin is and can do, see [Plugins](plugins.md).

## 1. Make the crate

Add the WebAssembly target that plugins build for, then make a library crate:

```sh
rustup target add wasm32-wasip2
cargo new --lib balance
cd balance
```

Replace `Cargo.toml` with this:

```toml
[package]
name = "balance"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
nori-plugin-sdk = { git = "https://github.com/norifm/nori" }

[profile.release]
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

The SDK comes from nori's repository. `Cargo.lock` pins the commit that your plugin builds against.

## 2. Write the sound stage

Replace `src/lib.rs` with this:

```rust
use nori_plugin_sdk::sound_stage::{Format, Inputs};
use nori_plugin_sdk::{Manifest, Row, Slot, SoundStage};

#[derive(Default)]
struct Balance {
    channels: u8,
    /// -1 is all left, 0 the middle, 1 all right.
    balance: f32,
}

impl SoundStage for Balance {
    fn configure(&mut self, format: Format, inputs: Inputs) {
        self.channels = format.channels;
        self.balance = inputs.rows.first().copied().unwrap_or(0.0) as f32;
    }

    fn process(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        if self.channels != 2 {
            return samples;
        }
        let frames = samples.len() / 2;
        let (left, right) = samples.split_at_mut(frames);
        let (gain_left, gain_right) = ((1.0 - self.balance).min(1.0), (1.0 + self.balance).min(1.0));
        left.iter_mut().for_each(|s| *s *= gain_left);
        right.iter_mut().for_each(|s| *s *= gain_right);
        samples
    }
}

const MANIFEST: Manifest = Manifest {
    stage: Some(Slot::Master),
    rows: &[Row::level("balance", -1.0, 1.0, 0.05, 0.0)],
    ..Manifest::new("com.example.balance")
};

nori_plugin_sdk::plugin!(Balance, MANIFEST, sound_stage);
```

nori calls `configure` with the format and the values of the plugin's rows, and again each time a value
changes. It calls `process` with each block of samples, the left channel first and then the right. The
manifest puts the stage in the master slot, after nori's equalizer. It gives the plugin one row, a
slider from -1 to 1 that starts at 0. The id is reverse DNS: use a domain of your own.

Balance keeps no state between blocks, so it needs no `save` or `restore`. The trait's other methods
answer as nori would without them.

## 3. Add its words

Make `strings/en.toml`, beside `Cargo.toml`:

```toml
name = "Balance"
about = "Moves the music to the left or the right."
balance = "Balance"
```

`name` and `about` are what nori lists. Each other key is a row's id, and its text is that row's label.

## 4. Build it

```sh
cargo build --release --target wasm32-wasip2
```

The first build fetches nori's repository for the SDK, which takes a few minutes. Cargo then writes
`target/wasm32-wasip2/release/balance.wasm`.

Always build with `--target wasm32-wasip2`. A build for your computer's own target fails to link,
because the plugin's export names are WebAssembly's.

## 5. Put the plugin in a folder

nori installs a plugin from a folder that holds `plugin.wasm` and `strings/`:

```sh
mkdir -p package/strings
cp target/wasm32-wasip2/release/balance.wasm package/plugin.wasm
cp strings/en.toml package/strings/
```

`ls -R package` shows `plugin.wasm` and `strings/en.toml`.

## 6. Install it in the desktop app

1. In the desktop app, open **Settings** and click the **Plugins** tab.
2. Click **Get plugins**, then **Install from file**.
3. Enter the full path of your `package` folder.

   nori asks "Install Balance 0.1.0?". Under the question, the sheet says "It’s a sound effect.",
   "Unsigned" and "It turns on once installed."

   If nori says "Not installed: can’t be read (no plugin.wasm)", check the path and the folder's files.
   "no nori:plugin section" means that the wasm was built without the `plugin!` line.
4. Click **Install**.

   Balance shows in the **Plugins** tab, turned on.

## 7. Hear it

1. Play a song.
2. In the **Plugins** tab, click **Balance**.

   Its page shows the **Balance** slider at 0.
3. Drag the slider all the way to the left.

   Within a moment, the music plays from the left side only.

If you hear no change, check that the song plays in stereo: Balance passes other formats through. If the
plugin fails, nori bypasses its stage for that song and writes why to its log, in lines that start with
`plugin com.example.balance:`. On Linux and macOS the log is `~/.local/share/nori/logs/nori.log`. On
Windows it is `%LOCALAPPDATA%\nori\logs\nori.log`.

## 8. Add a setting and update the plugin

Add a switch that swaps the two sides.

1. In `src/lib.rs`, add the row after the slider:

   ```rust
   rows: &[Row::level("balance", -1.0, 1.0, 0.05, 0.0), Row::switch("swap", false)],
   ```

2. Add a field `swap: bool` to `Balance`. In `configure`, read the second row: a switch is 0 or 1.

   ```rust
   self.swap = inputs.rows.get(1).copied().unwrap_or(0.0) != 0.0;
   ```

3. In `process`, after `split_at_mut`, swap the channels when the switch is on:

   ```rust
   if self.swap {
       left.swap_with_slice(right);
   }
   ```

4. Add the switch's label to `strings/en.toml`:

   ```toml
   swap = "Swap sides"
   ```

5. In `Cargo.toml`, set `version = "0.2.0"`. nori reads the plugin's version from there.
6. Build again and copy the files, as in steps 4 and 5.
7. In the desktop app, install the `package` folder again, as in step 6.

   The sheet says "Install Balance 0.2.0?". After you click **Install**, Balance's page shows the
   **Balance** slider where you left it, and the new **Swap sides** switch.

An update keeps the plugin's values, its place among the plugins and whether it is on.

## Next steps

- [Plugin reference](plugin-reference.md): every manifest field, row kind and limit. It also says how to
  install a plugin by hand on Android, in the terminal app and in a browser.
- [Plugins](plugins.md): the other extension points, and what a plugin can reach.
- `orbit` in [nori-plugins](https://github.com/norifm/nori-plugins) is a sound stage that keeps state
  between blocks with `save` and `restore`. The repo's `tools/pack.py` packs a plugin into a
  `.noriplugin` file for a release.
