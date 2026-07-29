# WindowMod (WaylandCraft Win32 capture)

Windows-порт [WaylandCraft](https://github.com/EVV1E/waylandcraft) — мод Fabric для Minecraft **26.1.2**, который отображает окна реальных приложений внутри игрового мира.

На Linux по-прежнему используется оригинальный Wayland-композитор (Smithay). На Windows — нативный слой **Win32**: захват содержимого HWND, запуск приложений из меню «Пуск» и пересылка ввода через Win32 API.

## Требования

- **Windows 10/11** (x64) или **Linux** (оригинальное поведение)
- Minecraft **26.1.2** + Fabric Loader **0.19.2+**
- **Java 25**
- **Rust** (для сборки native-библиотеки)
- Рекомендуется: Sodium, Iris (поддержка шейдеров сохранена)

### Linux-only (без изменений)

- xkbcommon, xwayland-satellite и т.д. — см. оригинальный README

## Управление

| Клавиша | Действие |
|---------|----------|
| **V** | Лаунчер приложений |
| **G** | Захват клавиатуры (Esc отпускает) |
| **B** | Менеджер окон |
| **Alt+Q** | Жёсткий захват клавиатуры + относительная мышь |

## Сборка (Windows)

```bat
build.bat
```

Или по шагам:

```bat
cd native
cargo build --release
cd ..
gradlew.bat build
```

JAR: `build\libs\waylandcraft-2.1.0-windowmod.jar`

### Запуск из IDE / dev-клиента

```bat
gradlew.bat runClient
```

Native DLL должна быть в `native\target\debug\waylandcraft.dll` (Gradle подхватывает её в JAR автоматически).

## Архитектура

```
Minecraft (Java Fabric)
    └── WaylandCraftBridge (JNI)
            ├── Linux: Smithay Wayland compositor + EGL/dmabuf
            └── Windows: WindowMod (Win32)
                    ├── capture.rs   — PrintWindow захват скрытых HWND
                    ├── input.rs     — PostMessage (мышь/клавиатура) в скрытые окна
                    ├── apps.rs      — сканирование .lnk из Start Menu
                    └── process.rs   — CreateProcessW + SW_HIDE + poll_pending_launches
```

Java-слой (рендер, window items, grabs, Iris) **общий** для обеих платформ.

## Что работает на Windows

- Лаунчер приложений (ярлыки Start Menu + Notepad, Calc, cmd, Paint)
- Захват и отображение окон в 3D / WM-экранe / предмете «Window»
- Мышь и клавиатура (в т.ч. Alt+Q hard capture) — через PostMessageW в скрытые окна
- Менеджер окон, resize/maximize (через SetWindowPos на -32000,-32000)
- Iris / framebuffer pipeline
- Мультиплеер: window items (как в оригинале — локально для игрока)

## Ограничения Windows vs Linux

| Функция | Linux | Windows |
|---------|-------|---------|
| Wayland / X11 клиенты | ✅ | ❌ (только Win32 HWND) |
| xwayland-satellite | ✅ | ❌ |
| Popup-окна (меню) | ✅ | ❌ (не реализовано) |
| Drag-and-drop | ✅ | ❌ |
| dmabuf / zero-copy | ✅ | ❌ (только SHM/PrintWindow) |
| XKB keymap export | ✅ | ❌ |
| Захват GPU-ускоренных окон | частично | ⚠️ PrintWindow может давать чёрный экран для DirectX/Vulkan |
| UWP / Store apps | N/A | ⚠️ ограниченно |

## Известные проблемы

1. **Чёрное/пустое окно** — некоторые приложения не рисуются через PrintWindow; попробуйте Notepad или Paint для проверки.
2. **Ввод** — PostMessage не всегда эквивалентен реальному фокусу; кликните по окну в игре перед набором текста.
3. **Права администратора** — приложения с elevated UI могут не захватываться из обычного Minecraft.
4. **Производительность** — покадровый CPU-захват медленнее Wayland dmabuf.

## Структура проекта

```
D:\WindowMod\
├── native/                 # Rust JNI (linux + windows)
│   ├── src/compositor.rs   # Linux Wayland
│   └── src/windows/        # Windows Win32 backend
│       ├── state.rs        # WindowMod, WinToplevel, WinSurface, PendingLaunch
│       ├── process.rs      # CreateProcessW, poll_pending_launches (hide+find window)
│       ├── capture.rs      # PrintWindow, refresh_windows, register_external_hwnd
│       ├── input.rs        # PostMessageW mouse/keyboard forwarding
│       ├── bridge.rs       # JNI bridge functions (exec_app, update, etc.)
│       └── apps.rs         # Lnk parsing, DesktopApp, Start Menu scan
├── src/main/java/          # Fabric mod (общий Java)
├── build.gradle
├── build.bat               # Windows build script
└── README.md               # этот файл
```

## Лицензия

GPLv3 — как оригинальный WaylandCraft. См. `LICENSE`.

## Благодарности

Оригинальный мод: [EVV1E/waylandcraft](https://github.com/EVV1E/waylandcraft)

---

# Win32 Hidden Window Implementation (Technical)

## Core Problem

Windows launched from the in-game launcher (V key) appeared on the Windows desktop instead of only rendering as 3D planes inside Minecraft.

**Root causes that were fixed:**
1. No `SW_HIDE` at process creation — window appeared on desktop before code could hide it
2. Tight polling (`std::thread::sleep`) on the render thread — caused lag and insufficient detection
3. Launcher processes (e.g., `javacpl.exe` spawns `javaw.exe` and exits) — PID tracking failed, real window had different PID
4. `WaitForInputIdle` blocked the render thread for seconds
5. Hint matching by executable name failed when window title didn't match (e.g., "javacpl" ≠ "Java Control Panel")
6. No HWND snapshot persisted — per-frame polling couldn't compare against pre-launch state

## Architecture: Window Launch → Detection → Hidden Capture

### 1. Process Creation (process.rs)

`spawn_executable()` in `native/src/windows/process.rs`:

1. Takes a HWND snapshot via `EnumWindows` collecting all current HWNDs as `HashSet<isize>`
2. Calls `CreateProcessW` with `STARTUPINFOW` containing:
   - `dwFlags: STARTF_USESHOWWINDOW`
   - `wShowWindow: SW_HIDE.0 as u16` (0 = SW_HIDE)
3. Returns immediately — **no blocking**, no `std::thread::sleep`, no `WaitForInputIdle`
4. Pushes a `PendingLaunch` struct with `pid`, `snapshot`, name hints, alt hint
5. If `CreateProcessW` fails, falls back to `ShellExecuteW` with `SW_HIDE`

`STARTF_USESHOWWINDOW` tells Windows to pass `SW_HIDE` as `nCmdShow` to the new process's `WinMain`. Well-behaved apps call `ShowWindow(nCmdShow)` which makes the window hidden from birth.

**Import note**: `STARTF_USESHOWWINDOW` is in `windows::Win32::System::Threading`, NOT in `WindowsAndMessaging`.

### 2. Per-Frame Detection (process.rs)

`poll_pending_launches()` is called every frame from `state.update()`. For each `PendingLaunch`:

**Triple search** (in order, returns first match):
1. **PID match** (`find_main_window_for_pid`) — `EnumWindows` + `GetWindowThreadProcessId`, skips `WS_CHILD`
2. **Snapshot comparison** (`find_new_window`) — `EnumWindows`, returns first HWND not in pre-launch snapshot. Catches any new window regardless of PID (solves launcher-process pattern)
3. **Name/path hints** (`find_by_hint`) — `EnumWindows` + `GetWindowTextW`, matches title with `contains()` against app name and executable stem

When found: `make_compositor_window()` (hides + moves to -32000,-32000) + `register_external_hwnd()` (creates WinToplevel + WinSurface).

**PendingLaunch stays alive for 400 frames** (~6.4s) to catch additional windows from the same launch (e.g., splash + main dialog). `register_external_hwnd` now checks for duplicate HWNDs.

### 3. Hidden Window Capture (capture.rs)

Uses `PrintWindow` via raw FFI:

```rust
#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc_dest: HDC, flags: u32) -> BOOL;
}
```

`PrintWindow` with `PW_CLIENTONLY = 1` sends `WM_PRINTCLIENT` to the window, which renders into a memory DC regardless of visibility. Works on hidden windows.

Pipeline: `PrintWindow` → `CreateCompatibleDC` → `CreateCompatibleBitmap` → `GetDIBits` (BGRA 32bpp, top-down with negative height) → pixel buffer → Java surface via SHM → 3D texture.

### 4. Per-Frame Safety (process.rs)

`ensure_windows_hidden()` iterates `state.toplevels` and calls `ShowWindow(hwnd, SW_HIDE)` every frame. This prevents apps from re-showing themselves (common with Java/Swing apps).

### 5. Input Forwarding (input.rs)

All mouse/keyboard input uses `PostMessageW`:
- `WM_MOUSEMOVE`, `WM_LBUTTONDOWN`, `WM_LBUTTONUP`, etc. — with packed coordinates in `LPARAM`
- `WM_KEYDOWN`, `WM_KEYUP` — with scancode→vk mapping via `MapVirtualKeyW`
- **No `SetForegroundWindow`** — removed because it would make hidden windows visible

### 6. Off-Screen Positioning (bridge.rs)

`resize_hwnd()` uses `SetWindowPos(hwnd, HWND_TOP, -32000, -32000, width, height, SWP_NOZORDER)`. All resize/maximize/fullscreen operations position the window off-screen.

## Edge Cases Handled

| Case | Solution |
|------|----------|
| Launcher process spawns child with different PID | Snapshot comparison catches any new HWND |
| App calls ShowWindow(SW_SHOW) after being hidden | `ensure_windows_hidden` re-hides every frame |
| Multiple windows per launch (splash + main) | PendingLaunch kept alive 400 frames |
| Same HWND found multiple times | `register_external_hwnd` checks `state.toplevels` for duplicate |
| Malformed .lnk files in Start Menu | Panics caught per-file, skipped with warning |
| CreateProcessW fails (path, permissions) | Falls back to ShellExecuteW with SW_HIDE |
| Process takes seconds to create window | Per-frame polling for up to 400 frames |

## Key Files

### `native/src/windows/state.rs`
- `PendingLaunch` — pid, app_id, attempts, hint, alt_hint, snapshot (HashSet of HWND isize)
- `WindowMod` — toplevels, surfaces, pending_launches, + state
- `ptr_of` / `ptr_to_ref` / `ptr_to_mut` — raw pointer conversion for Java jlong handles
- `retain_toplevels()` — removes dead windows each frame via `hwnd_alive(IsWindow)`

### `native/src/windows/process.rs`
- `spawn_app()`/`spawn_desktop_app()` — entry points from JNI
- `spawn_executable()` — CreateProcessW with SW_HIDE, snapshot, push PendingLaunch
- `fallback_shellexec()` — ShellExecuteW with SW_HIDE
- `poll_pending_launches()` — per-frame triple search
- `make_compositor_window()` — SW_HIDE + SetWindowPos(-32000, -32000)
- `ensure_windows_hidden()` — per-frame re-hide all managed windows
- `snapshot_hwnds()` / `find_new_window()` — EnumWindows helpers
- `find_new_window_by_hint()` / `find_by_hint()` — window title matching

### `native/src/windows/capture.rs`
- `refresh_windows()` — iterate toplevels, capture via PrintWindow, mark surfaces dirty
- `register_external_hwnd()` — create WinToplevel + WinSurface, skip duplicates
- `find_main_window_for_pid()` — EnumWindows + GetWindowThreadProcessId
- `hwnd_alive()` — IsWindow

### `native/src/windows/input.rs`
- `pointer_motion()` / `pointer_motion_focus()` / `pointer_button()` / `pointer_axis()`
- `keyboard_key()` / `focus_toplevel()` / `minimize_toplevel()`
- All use `PostMessageW` — no `SetForegroundWindow`

### `native/src/windows/bridge.rs`
- JNI functions: `exec_app()`, `update()`, `toplevel_resize()`, etc.
- `resize_hwnd()` — off-screen positioning at (-32000, -32000)

## Potential Issues for Next Iteration

1. **DLL loading from jar fails in dev**: `getResourceAsStream("/waylandcraft.dll")` returns null in dev because the DLL isn't in `src/main/resources/`. Dev falls through to `System.loadLibrary("waylandcraft")` from PATH. Fix: copy DLL to resources during build, or fix the resource path so JAR loading works in dev.

2. **PrintWindow limitations**: GPU-rendered content (DirectX, OpenGL, Vulkan) may render blank/black via PrintWindow. WM_PRINTCLIENT renders via GDI only. For GPU content, DXGI capture (`IDXGIOutputDuplication`) would be needed.

3. **Per-frame EnumWindows cost**: `poll_pending_launches` calls `EnumWindows` multiple times per frame. On systems with hundreds of HWNDs, this may have performance impact. Could add rate-limiting.

4. **Window title changes**: Hint matching uses `contains()` at the moment of search. After registration, `refresh_windows` updates `toplevel.title` each frame via `GetWindowTextW`.

5. **Thread safety**: All Rust state is behind `&mut WindowMod` — single-threaded from the render thread. No locks or atomics. Would break if JNI called from multiple threads.

6. **lnk crate panics**: `lnk-0.5.1` panics on some .lnk files with `range start index out of bounds`. Panics are caught per-file. No Rayon parallel iteration.

---

# Сессия 2026-06-29 — Исправление desktop flash + input + производительность

## Что было сделано

### 1. Захват: WS_EX_TOOLWINDOW + off-screen (process.rs)

`make_compositor_window()` переписана:
- Убран `ShowWindow(SW_HIDE)` — ломает PrintWindow (возвращает нули для скрытых окон)
- Оставлен только `WS_EX_TOOLWINDOW` + `SetWindowPos(-32000,-32000)`
- Результат: окно невидимо на рабочем столе, PrintWindow корректно захватывает содержимое

**Подтверждено**: 1428x747 пикселей с валидными данными `[32, 32, 32, 255, ...]`

### 2. Фильтрация зомби-окон (process.rs, find_new_window_by_hint)

Добавлена проверка `WS_EX_TOOLWINDOW` в `find_new_window_by_hint()`:
- Зомби-окна с прошлых сессий (которым уже был применён `WS_EX_TOOLWINDOW`) теперь пропускаются при поиске новых окон
- Дополняет существующий фильтр `WS_EX_LAYERED`

### 3. Polling loop после CreateProcessW (process.rs, spawn_executable)

Заменён бесполезный immediate scan на цикл с Sleep:
```rust
for attempt in 0..50 {
    std::thread::sleep(Duration::from_millis(10));
    if let Some(hwnd) = find_new_window_safe(&snapshot) {
        make_compositor_window(hwnd);
        break;
    }
}
```
- `CreateProcessW` асинхронный — HWND может не существовать сразу
- Максимум 500мс ожидания, опрос каждые 10мс
- При обнаружении сразу прячет окно через `make_compositor_window`

### 4. Клавиатура: ToUnicodeEx вместо MapVirtualKeyW (input.rs)

Заменено `MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR)` на `ToUnicodeEx()`:
- Старый код: работал только для латинских WASD клавиш
- Новый код: корректно маппит символы по активной раскладке клавиатуры (русская, и т.д.)
- Добавлены extern-объявления для `ToUnicodeEx`, `GetKeyboardState`, `GetKeyboardLayout`
- `GetKeyboardLayout(0)` возвращает раскладку текущего потока

### 5. Мышь: with_foreground для кликов (input.rs)

`pointer_button()` по-прежнему использует `with_foreground()`:
- `AttachThreadInput` → `SetForegroundWindow(hwnd)` → `PostMessageW(WM_LBUTTONDOWN)` → `SetForegroundWindow(mc)`
- Необходим для обработки кликов в Notepad/других приложениях

## Текущее состояние

### Что работает
- Захват PrintWindow: 1428x747 с валидными пикселями
- Отображение в Minecraft (в мире + в руке)
- `WS_EX_TOOLWINDOW` + off-screen скрывает окно от десктопа
- Фильтрация зомби-окон
- ToUnicodeEx для клавиатуры
- `with_foreground` для мыши

### Оставшиеся проблемы

1. **Окна появляются на рабочем столе**: Между `CreateProcessW` и обнаружением окна (polling loop) проходит до 500мс. За это время окно может мелькнуть на десктопе. Причины:
   - `SW_HIDE` в `STARTUPINFOW` ненадёжен (Win11 Notepad вызывает `ShowWindow(SW_SHOW)` при инициализации)
   - Окно создаётся асинхронно — нет способа гарантированно скрыть его до появления
   - Возможное решение: `EnumWindows` polling в отдельном потоке вместо `std::thread::sleep` на render thread

2. **Зомби-процессы не убиваются**: `Stop-Process -Force` не всегда убивает Notepad из прошлых сессий. Накапливаются и вызывают:
   - Лаг: каждый зомби захватывается PrintWindow каждый кадр (1428x747)
   - Ложные срабатывания обнаружения
   - Нужно: cleanup-механизм при старте или `taskkill /F /IM notepad.exe`

3. **Сон на render thread**: `std::thread::sleep(10ms)` × 50 в `spawn_executable` блокирует render thread до 500мс. Нужно вынести в отдельный поток.

4. **Производительность PrintWindow**: Каждый кадр создаёт `CreateCompatibleDC` + `CreateCompatibleBitmap` + `GetDIBits` для каждого toplevel. Это CPU-bound операция на render thread. Нужно: кеширование DC/BITMAP или DXGI захват.

## Бэкапы

- `backup_native_src_2026-06-29_1604.zip` — до изменений сессии
- `backup_native_src_2026-06-29_17-05.zip` — текущее состояние
