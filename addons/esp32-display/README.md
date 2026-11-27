# ESP32 display

A small screen next to your PC showing what Wavy is playing: title, artist, progress, play/pause and like. Three buttons control the player.

| Button | Short press | Hold |
|---|---|---|
| left | previous track | volume down (repeats) |
| middle | play / pause | like / unlike |
| right | next track | volume up (repeats) |

It has two parts:

- **This addon** runs on the PC with Wavy. Wavy's API only answers the PC itself, so the addon relays it to your home network.
- **The sketch** (`wavy_display/wavy_display.ino`) runs on the ESP32. It finds the PC by itself, so you don't need its IP address.

## What to buy

| Part | Notes |
|---|---|
| ESP32 dev board | Any "ESP32 DevKit" / "ESP32-WROOM-32" board with a USB port. ESP32-S3/C3 boards work too, but the pin numbers differ. |
| 0.96" or 1.3" I²C OLED, 128×64 | The 4-pin kind (GND, VCC, SCL, SDA). 0.96" is usually SSD1306, 1.3" is usually SH1106. |
| 3 push buttons | The common 6×6 mm tactile buttons. |
| Breadboard and jumper wires | Or solder everything onto perfboard. |
| USB cable | It also powers the ESP32. Plug it into the PC or any phone charger. |

## Wiring

```
OLED VCC  → 3V3            Left button   → GPIO 32 and GND
OLED GND  → GND            Middle button → GPIO 33 and GND
OLED SDA  → GPIO 21        Right button  → GPIO 25 and GND
OLED SCL  → GPIO 22
```

Each button goes between its pin and GND. No resistors are needed, because the ESP32's built-in pull-ups are used. Use **3V3** for the screen, not 5V/VIN.

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
5. Plug in the ESP32, pick **Tools → Board → esp32 → ESP32 Dev Module** and its port under **Tools → Port**, then press **Upload**. If the upload hangs at "Connecting…", hold the board's **BOOT** button until it starts.

The screen shows "Connecting to Wi-Fi…", then "Looking for Wavy…", and then the current track. **Tools → Serial Monitor** at 115200 baud shows the same steps.

## Troubleshooting

| Screen says | Fix |
|---|---|
| Connecting to Wi-Fi… | Check the name and password. The ESP32 only works with **2.4 GHz** Wi-Fi; most routers have both bands, so pick the 2.4 GHz one. |
| Looking for Wavy… | Is the addon ticked in Wavy, and is Wavy running? Some routers block broadcasts ("AP/client isolation", guest networks). Then set `PC_HOST` in the sketch to the PC's IP address (it's printed in `addon.log`). Giving the PC a fixed IP in your router settings keeps that working. |
| Connecting to PC… (over and over) | The firewall is blocking port 3739: see step 1.4. |
| Wrong key! | `KEY` in the sketch doesn't match `key` in `config.json`. |
| Nothing at all | Check the screen wiring and the SSD1306/SH1106 choice. Some screens swap the GND and VCC pins, so read the labels. |

Japanese and other scripts the screen's font can't draw are left out of titles. Accented letters lose their accents.

## Changing things

- **Pins:** `PIN_PREV`, `PIN_PLAY`, `PIN_NEXT` in the sketch. Avoid GPIO 34–39 (they have no pull-ups), and avoid 6–11 and 0/2/12/15, which the board uses for flash and booting.
- **Ports:** `port` and `discovery_port` in `config.json`, then change `PORT` and `DISCOVERY_PORT` in the sketch to match.
- **Several gadgets:** any number of ESP32s can connect at once with the same key.

## How it works

The PC sends one line per change, fields separated by tabs: `S playing liked volume position duration title artist`, or `N` when nothing is loaded. It sends at least every 5 s, so the ESP32 notices a lost link. The ESP32 sends `toggle`, `next`, `prev`, `like`, `vol+` or `vol-`. The key keeps other devices on the network from controlling your player. It's a simple shared secret, not encryption, so use it on a home network you trust.
