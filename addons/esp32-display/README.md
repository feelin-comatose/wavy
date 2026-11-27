# ESP32 display

A small screen next to your PC showing what Wavy is playing: title, artist, progress, play/pause and like. Four buttons control the player, and a tiny equalizer dances in the corner while music plays.

| Button | Short press | Hold |
|---|---|---|
| 1 | previous track | volume down (repeats) |
| 2 | play / pause | like / unlike |
| 3 | next track | volume up (repeats) |
| 4 | next screen | back to "Now playing" |
| knob | turn: volume (2% per click); press: mute / unmute | |

The fourth button switches between three screens:

- **Now playing:** title, artist, progress bar and times.
- **Up next:** the next three tracks in the queue. Buttons 1 and 3 (or turning the knob) move the highlight, button 2 (or pressing the knob) plays the highlighted track. It returns to "Now playing" after 20 s.
- **Clock:** the time (set from the internet), the date, and the current track along the bottom.

It has two parts:

- **This addon** runs on the PC with Wavy. Wavy's API only answers the PC itself, so the addon relays it to your home network.
- **The sketch** (`wavy_display/wavy_display.ino`) runs on the ESP32. It finds the PC by itself, so you don't need its IP address.

## What to buy

| Part | Notes |
|---|---|
| ESP32 dev board | Any "ESP32 DevKit" / "ESP32-WROOM-32" board, or a small ESP32-C3 board ("C3 SuperMini"). An ESP8266 (NodeMCU, Wemos D1 mini) works too, but without the fourth button. |
| 0.96" or 1.3" I²C OLED, 128×64 | The 4-pin kind (GND, VCC, SCL, SDA). 0.96" is usually SSD1306, 1.3" is usually SH1106. |
| 4 push buttons | The common 6×6 mm tactile buttons. If your screen module has its own buttons with their own pins, wire those instead. |
| Rotary encoder (optional) | For the volume knob: a 5-pin module such as the KY-040 (CLK, DT, SW, +, GND). Not on the ESP8266. |
| Breadboard and jumper wires | Or solder everything onto perfboard. |
| USB cable | It also powers the ESP32. Plug it into the PC or any phone charger. |

## Wiring

OLED VCC goes to **3V3** (not 5V/VIN) and OLED GND to **GND**. The other pins depend on the board; the sketch picks the right set by itself:

| | OLED SDA | OLED SCL | Button 1 | Button 2 | Button 3 | Button 4 | Knob CLK | Knob DT | Knob SW |
|---|---|---|---|---|---|---|---|---|---|
| ESP32 | GPIO 21 | GPIO 22 | GPIO 32 | GPIO 33 | GPIO 25 | GPIO 26 | GPIO 27 | GPIO 14 | GPIO 13 |
| ESP32-C3 | GPIO 5 | GPIO 6 | GPIO 3 | GPIO 4 | GPIO 10 | GPIO 7 | GPIO 0 | GPIO 1 | GPIO 20 |
| ESP8266 | D2 | D1 | D5 | D6 | D7 | — | — | — | — |

The knob's **+** pin goes to 3V3 and its **GND** to GND. If one click changes the volume twice, set `ENC_STEPS` to 2 in the sketch; if turning right lowers the volume, set `ENC_REVERSE` to `true`.

Each button goes between its pin and GND. No resistors are needed, because the board's built-in pull-ups are used. On small C3 boards the pins are labelled with plain numbers, often on the back.

## 1. Set up the addon on the PC

1. Install [Python 3](https://www.python.org/downloads/). On Windows, tick **"Add python.exe to PATH"** in the installer.
2. In Wavy, open **Settings → Addons → Open addons folder** and copy this whole `esp32-display` folder into it.
3. Press **Refresh list** and tick **ESP32 display**.
4. **Windows:** a firewall prompt for Python appears. Allow it on **Private networks**. If you missed the prompt, run this once in PowerShell as administrator:
   ```powershell
   New-NetFirewallRule -DisplayName "Wavy ESP32" -Direction Inbound -Protocol TCP -LocalPort 3739 -Action Allow -Profile Private
   ```
   Also make sure your home Wi-Fi is set as a **Private** network (Settings → Network → your network → Network profile type).
5. Open `config.json` in the addon folder. On the first start the addon puts a random `key` there, and it's also printed in `addon.log`. You'll paste it into the sketch.

## 2. Flash the ESP32

1. Install the [Arduino IDE](https://www.arduino.cc/en/software) (version 2).
2. Add ESP32 support: **File → Preferences → Additional boards manager URLs**:
   ```
   https://espressif.github.io/arduino-esp32/package_esp32_index.json
   ```
   Then **Tools → Board → Boards Manager**, search for **esp32** and install **esp32 by Espressif Systems**.
3. Install the display library: **Tools → Manage Libraries**, search for **U8g2** and install **U8g2 by oliver**.
4. Open `wavy_display/wavy_display.ino` and fill in the settings at the top:
   ```cpp
   const char* WIFI_SSID = "your-wifi-name";
   const char* WIFI_PASS = "your-wifi-password";
   const char* KEY       = "the key from config.json";
   ```
   For a 1.3" SH1106 screen, comment out the `SSD1306` line and uncomment the `SH1106` line below it.
   The clock uses UK time. For another timezone, change `TIMEZONE` (examples are next to it).
5. Plug in the board, pick it under **Tools → Board** and its port under **Tools → Port**, then press **Upload**. If the upload hangs at "Connecting…", hold the board's **BOOT** button until it starts.
   - ESP32: **esp32 → ESP32 Dev Module**.
   - ESP32-C3: **esp32 → ESP32C3 Dev Module**, and set **Tools → USB CDC On Boot → Enabled** so the Serial Monitor works.
   - ESP8266: add `https://arduino.esp8266.com/stable/package_esp8266com_index.json` to the boards manager URLs, install **esp8266**, then pick **NodeMCU 1.0** (or your board).

The screen shows "Connecting to Wi-Fi…", then "Looking for Wavy…", and then the current track. **Tools → Serial Monitor** at 115200 baud shows the same steps.

## Troubleshooting

| Screen says | Fix |
|---|---|
| Connecting to Wi-Fi… | Check the name and password. These boards only work with **2.4 GHz** Wi-Fi; most routers have both bands, so pick the 2.4 GHz one (a name ending in `_5` or `-5G` is usually 5 GHz). The small antennas also need a decent signal: move the board closer to the router to test. |
| Looking for Wavy… | Is the addon ticked in Wavy, and is Wavy running? Some routers block broadcasts ("AP/client isolation", guest networks). Then set `PC_HOST` in the sketch to the PC's IP address (it's printed in `addon.log`). Giving the PC a fixed IP in your router settings keeps that working. |
| Connecting to PC… (over and over) | The firewall is blocking port 3739: see step 1.4. |
| Wrong key! | `KEY` in the sketch doesn't match `key` in `config.json`. |
| Nothing at all | Check the screen wiring and the SSD1306/SH1106 choice. Some screens swap the GND and VCC pins, so read the labels. |
| Up next: Queue is empty | Update this addon too (the queue needs its newer version), then switch it off and on in Settings → Addons. |
| Clock: --:-- | The time comes from the internet and takes up to a minute after start-up. |

Japanese and other scripts the screen's font can't draw are left out of titles. Accented letters lose their accents.

## Changing things

- **Pins:** `PIN_PREV`, `PIN_PLAY`, `PIN_NEXT`, `PIN_VIEW` in the sketch, in the section for your board. On the ESP32 avoid GPIO 34–39 (no pull-ups) and 6–11, 0, 2, 12 and 15 (flash and booting). On the C3 avoid 2, 8, 9 (booting) and 18, 19 (USB).
- **Ports:** `port` and `discovery_port` in `config.json`, then change `PORT` and `DISCOVERY_PORT` in the sketch to match.
- **Several gadgets:** any number of ESP32s can connect at once with the same key.

## How it works

The PC sends one line per change, fields separated by tabs: `S playing liked volume position duration title artist`, or `N` when nothing is loaded, followed by `Q` and then index, title and artist for each of the next three tracks. It sends at least every 5 s, so the ESP32 notices a lost link. The ESP32 sends `toggle`, `next`, `prev`, `like`, `vol+`, `vol-`, `vol <change in %>`, `mute` or `jump <queue index>`. The key keeps other devices on the network from controlling your player. It's a simple shared secret, not encryption, so use it on a home network you trust.
