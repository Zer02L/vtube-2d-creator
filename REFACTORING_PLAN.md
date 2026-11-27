# План рефакторинга `creator-2d-avatar`

Документ для постепенного выполнения. Отмечай выполненное галочкой: `- [x] ...`.
Принцип: сначала гигиена и выделение общего кода **без изменения поведения**, затем разделение крупных файлов, и только потом функциональные доработки.

Легенда:
- `[ ]` — не начато
- `[x]` — сделано и проверено (`pnpm run build` / `cargo test` / ручная проверка)
- `[~]` — в работе (можно писать так, потом заменить на `[x]`)

## Исходная картина (на момент составления)

| Файл | Строк | Проблема |
|---|---:|---|
| `src/components/SeamlessLooper.vue` | 1390 | Скрипт ~740 + шаблон ~650; 20+ ref, 3 способа загрузки, 4 canvas-рендера, 2 экспортера |
| `src-tauri/src/looper/mod.rs` | 1085 | DTO + ffprobe + математика + граф ffmpeg + раннер + тесты |
| `src/utils/loopEngine.ts` | 1011 | Извлечение кадров, математика, playback-план, рендер, 2 MediaRecorder-экспортера |
| `src/components/ChromaOptimizer.vue` | 821 | Два preview-цикла, кэш, загрузка, экспорт, результат |
| `src-tauri/src/bin/loop-video.rs` | 293 | Ручной парсер CLI |
| `src/utils/chroma.ts` | 291 | Свои `waitForMeta/seekTo/waitForPresentedFrame` вместо общих |
| `src-tauri/src/optimizer/mod.rs` | 388 | Раннер ffmpeg, дублирующий looper |

Топ-4 файла = **4107 строк ≈ 63%** всего кода (~6476 LOC).
Цель после Фаз 1–3: топ-4 → ~1500–2000 строк, один источник правды для алгоритмов и UI-паттернов.

---

## Фаза 0. Гигиена и мёртвый код (≈1 час, без риска)

- [ ] Удалить 3 закоммиченных тестовых `.webm` из корня `src-tauri/` (~9.5 МБ). При необходимости нужные файлы перенести в `tests/fixtures/`.
- [ ] Добавить в `.gitignore` правило для тестовых медиа (например `tests/fixtures/*.webm` или `*.webm` в `src-tauri/`).
- [ ] Удалить неиспользуемую зависимость `@vueuse/core` из `package.json` + `pnpm-lock.yaml` (`pnpm remove @vueuse/core`).
- [ ] Добавить `@types/node` в devDependencies и убрать `// @ts-expect-error` над `import process from 'node:process'` в `vite.config.ts`.
- [ ] Убрать `'profile'` из union-типа `activeTab` в `src/App.vue:8` (вкладки и контента нет).
- [ ] Разобраться с мёртвыми Tauri-командами в `src-tauri/src/lib.rs`: `get_obs_url:41`, `preview_ffmpeg_args:46`. Либо подключить к UI, либо удалить из `generate_handler!` и кода.
- [ ] Убрать статичный индикатор «OBS Сервер активен» в `src/App.vue:58` или завязать его на реальное состояние (`store.isServerRunning`).
- [ ] Удалить неиспользуемые поля `isServerRunning`, `isOverlayOpen` из `src/stores/avatar.ts`, если они не понадобятся в Фазе 4 (иначе оставить с пометкой).
- [ ] Привести `README.md` в соответствие с фактом: HTTP-сервера, загрузки/сохранения профилей и работающего оверлея сейчас нет. Либо удалить обещания, либо оставить TODO-списком.
- [ ] Проверить: `pnpm run build` проходит, `git status` чистый.

## Фаза 1. Общие модули frontend (чистый move-код)

Создать каталог `src/lib/` (или `src/utils/` — выбрать один стиль):

- [ ] `src/lib/media/videoCapture.ts` — скрытый `<video>` (одинаковые стили в `loopEngine.ts:172` и `chroma.ts:183`), тип `RvfcVideo`, `waitForEvent`/`waitForMeta`, `seekVideo`/`seekTo`, `captureFrames`, `snapshotFrame`, детектор пустого кадра. Экспортировать одну надёжную функцию извлечения кадров.
- [ ] `src/lib/media/canvasRecorder.ts` — выбрать поддерживаемый MIME для WebM, создать `MediaRecorder`, покадрово отрисовывать через callback, собрать `Blob` + `URL.createObjectURL`. Объединить `loopEngine.ts:697-894` и `loopEngine.ts:912-1011`.
- [ ] `src/lib/video/similarity.ts` — `computeFrameSimilarity`, `meanAbsDiffNorm`, `motionDiffNorm`, `analyzeFramesInBrowser` (эквивалент Rust `find_best_candidates`).
- [ ] `src/lib/video/playbackPlan.ts` — `buildLoopPlaybackPlan` + типы `LoopPlaybackStep`.
- [ ] `src/lib/video/render.ts` — `renderBlendedFrame`, `renderSeamComparison` (onion/split/diff).
- [ ] `src/lib/video/presets.ts` — пресеты разрешений, `resolveResolutionDims`, `parseResolutionDims`, нормализация hex-цвета, `guessMimeFromPath`. Устранить 5 копий из `ChromaOptimizer.vue:93`, `chroma.ts:47`, `loopEngine.ts:679`, `SeamlessLooper.vue:576` (и Rust-аналоги — см. Фазу 3).
- [ ] `src/lib/tauri/ipc.ts` — типизированные обёртки `invoke` + `Channel`, helpers `stageVideo(file)`, `readVideoBytes(path)`, `revealInFolder(path)`, `loadPreviewSheet(path)`. Сюда же переехать `guessMimeFromPath`.
- [ ] `src/types/avatar.ts` — ввести алиасы `ProgressEvent { stage, percent, message }` и `ChromaKeyParams` вместо двух идентичных интерфейсов и двух inline-форм `chromaKey` (строки 63, 100, 56, 93).
- [ ] Обновить импорты во всех потребителях; `pnpm run build` проходит; поведение не изменилось.

## Фаза 2. Разделение крупных компонентов

### SeamlessLooper.vue (1390 → orchestrator ~150 + 6–7 файлов)

- [ ] `useVideoSource.ts` — загрузка через `<input>`, browser drop, Tauri drag-drop, путь из `localInputPath`, `stage_uploaded_video` → `probe_video` → object URL, единая очистка ресурсов. Используется и в ChromaOptimizer.
- [ ] `useCanvasPlayback.ts` — RAF-цикл с `isPlaying`/скоростью/активностью таба (копии в `SeamlessLooper:342`, `ChromaOptimizer:161`, `ChromaOptimizer:202`, `DualPlayer:59`).
- [ ] `src/components/loop/LoopPreview.vue` — canvas цикла, бейджи «ШОВ ЦИКЛ / ПЕРЕХОД», кнопки паузы/скорости/«к стыку».
- [ ] `src/components/loop/SeamInspector.vue` — canvas сравнения первого/последнего кадра, режимы onion/split/diff, слайдер прозрачности, метрика схожести.
- [ ] `src/components/loop/LoopTimeline.vue` — дорожка, range-слайдеры start/end, список найденных кандидатов, кнопка автопоиска.
- [ ] `src/components/loop/LoopModeSelector.vue` — карточки 4 режимов и их параметры (crossfade / bridge / exact end frame).
- [ ] `src/components/loop/LoopExportSettings.vue` — формат, разрешение, FPS, хромакей, путь сохранения, кнопка экспорта, прогресс и ошибка.
- [ ] `src/components/loop/LoopResult.vue` — метрики, скачать/открыть папку, назначить на Idle/Talk.
- [ ] `SeamlessLooper.vue` оставить как тонкий оркестратор состояния.

### ChromaOptimizer.vue (821 → orchestrator + 3 файла)

- [ ] `src/components/chroma/ChromaPreview.vue` — превью хромакея, пипетка, слайдер времени, оверлей «ОРИГИНАЛ/ХРОМАКЕЙ», кэш обработанных кадров.
- [ ] `src/components/chroma/ChromaSettings.vue` — разрешение, цвет, similarity, blend, путь сохранения, кнопка конвертации.
- [ ] `src/components/chroma/ChromaResult.vue` — canvas-превью результата, метрики, скачать/папка/назначить (переиспользовать общий `ResultActions.vue` из loop).
- [ ] `src/components/common/VideoDropzone.vue` — единый drag&drop + `<input type=file>` + индикатор FFmpeg Native / Browser Canvas.
- [ ] `src/components/common/ProgressBar.vue`, `ErrorAlert.vue`, `AlphaCheckerboard.vue` (убрать 3 копии CSS-шахматки), `ResultActions.vue`, `ChromaControls.vue`.

## Фаза 3. Разделение Rust-бэкенда

- [ ] `src-tauri/src/looper/types.rs` — `LoopMode`, `OutputFormat`, `VideoProbeInfo`, `LoopCandidate`, `LoopAnalysisResult`, `Loop*Options/Result/Event`.
- [ ] `src-tauri/src/looper/probe.rs` — `probe_video`, `parse_rational_fps`, `pix_fmt_has_alpha`, `count_frames_fallback`, `input_decoder_args`, `measure_seam_similarity`.
- [ ] `src-tauri/src/looper/analysis.rs` — `frame_similarity_percent`, `mean_abs_diff_norm`, `motion_diff_norm`, `find_best_candidates`, `analyze_loop_points`.
- [ ] `src-tauri/src/looper/graph.rs` — `build_loop_ffmpeg_args` и вспомогательные функции (`derive_output_path`, `normalize_resolution`, codec-аргументы по форматам).
- [ ] `src-tauri/src/looper/process.rs` — `process_loop_video` и его тесты.
- [ ] `src-tauri/src/ffmpeg_common.rs` — общий `run_with_progress(args, expected_duration, on_progress)`: spawn ffmpeg, читать stderr в потоке, парсить `out_time_us=`, wait, ошибки. Использовать в `looper/process.rs` и `optimizer/mod.rs:209-319`.
- [ ] `src-tauri/src/common/paths.rs` — `AllowedVideoPaths`, `sanitize_file_name`, stage-загрузка во temp, `derive_output_path`. Общий normalize цвета/разрешения для looper и optimizer.
- [ ] `src-tauri/src/commands/*.rs` — тонкие Tauri-обработчики, `lib.rs` оставить только регистрацией (`generate_handler!`).
- [ ] CLI `src-tauri/src/bin/loop-video.rs`: переписать ручной парсер на `clap` derive (293 → ~100 строк, валидация и автогенерация `--help`). Добавить тесты на парсинг аргументов.
- [ ] `cargo test` проходит (в Nix: `nix-shell --run "cargo test"`; в обычном окружении нужен `pkg-config` + dbus-dev).

## Фаза 4. Функциональные дыры (решить, что делать с «Стрим и триггеры»)

- [ ] Принять решение: **(а)** сделать таб рабочим или **(б)** переименовать в «Демо» и убрать обещания в README. Рекомендуется (а).
- [ ] Наполнить `manifest.emotions` / `manifest.oneShots` данными вместо хардкода `sampleEmotions`/`sampleOneShots` в `ControlPanel.vue:7-18`; убрать хардкод.
- [ ] Подключить `resolveActiveVideoId` (`stores/avatar.ts:37`) к реальным видео: эмоции и one-shot должны менять `activeSlot` в `DualPlayer`.
- [ ] Реализовать загрузку/сохранение манифеста профиля (модуль `src-tauri/src/profile/mod.rs` уже есть, но не подключен): Tauri-команды `load_profile` / `save_profile` + файл `.vtuber` в `$APPDATA`.
- [ ] Перестать хранить `blob:` URL в манифесте: сохранять путь к файлу и восстанавливать объект URL при старте; иначе назначенные лупы теряются при перезапуске.
- [ ] Внедрить настоящий VAD: либо Rust `AudioEngine` (`src-tauri/src/audio/vad.rs`) + захват микрофона и стрим RMS в frontend через `Channel`, либо оставить JS, но реализовать attack/hold из Rust-логики в `setVolume` (`stores/avatar.ts:66`).
- [ ] Либо реализовать локальный HTTP-сервер на `localhost:12345/avatar` для Browser Source OBS, либо удалить URL/индикатор из UI и README.
- [ ] Подключить оверлей-окно из `tauri.conf.json`: команды открытия/закрытия, передача состояния аватара, настройки click-through.
- [ ] Проверить работу горячих клавиш (`Ctrl+1..4`, `F1..F3` — сейчас это только подписи на кнопках).

## Фаза 5. Надёжность, тесты, производительность

- [ ] Отмена длительных операций: `AbortController` для браузерного экспорта и kill ffmpeg-процесса по токену из Rust; кнопка «Стоп» в UI.
- [ ] Единый контракт алгоритма анализа: Rust — источник правды, зафиксировать тест-векторы и прогонять TS `analyzeFramesInBrowser` на них в Vitest, чтобы нативный и браузерный режимы не расходились.
- [ ] Vitest: тесты на `buildLoopPlaybackPlan` (все 4 режима + `exactEndFrame`), `computeFrameSimilarity`, `presets`, `canvasRecorder` mime-выбор.
- [ ] Расширить Rust-тесты: парсер `out_time_us`, `derive_output_path` для всех форматов, граничные случаи `build_loop_ffmpeg_args` (короткий отрезок, 1 кадр, alpha/no-alpha).
- [ ] E2E (Playwright): сценарий «загрузка файла → извлечение кадров → превью стыка → экспорт WebM» в браузерном режиме.
- [ ] Лимит размера загружаемого видео и очистка старых staged-файлов из temp по времени/квоте.
- [ ] Проверка allowlist в `reveal_in_folder` (`lib.rs:201`) — только пути, известные приложению.
- [ ] Защита от коллизий имён staged-файлов (`timestamp + sanitized name` в `stage_uploaded_video`): добавить случайный suffix.
- [ ] Оптимизация памяти превью: хранить кадры как `ImageBitmap` или один спрайтлист с region-draw вместо десятков canvas с полным RGBA (сейчас ~десятки МБ на 480p×100 кадров).
- [ ] Явно обозначить в UI, что браузерный MediaRecorder не сохраняет альфу; возможно, сделать браузерный режим только для превью.

---

## Рекомендованный порядок запуска

1. **Фаза 0** целиком — быстро и безопасно.
2. Фаза 1: `videoCapture.ts` → `canvasRecorder.ts` → `presets.ts` → `ipc.ts` → типы.
3. Фаза 2: `useVideoSource.ts` → `useCanvasPlayback.ts` → разбиение `SeamlessLooper.vue`.
4. Фаза 3: `ffmpeg_common.rs` → разбиение `looper/mod.rs` → `clap` в CLI.
5. Фаза 4 начинать только после решения по продукту (нужен ли реальный стриминг-режим).
6. Фаза 5 — непрерывно по мере возможности; тесты писать вместе с каждым новым модулем.

## Критерий завершения фазы

- Все чекбоксы фазы отмечены `[x]`;
- `pnpm run build` проходит без ошибок `vue-tsc`;
- `nix-shell --run "cargo test"` проходит;
- Вручную проверены оба режима: Tauri (FFmpeg native) и браузерный (Canvas fallback);
- Поведение пользователя до/после рефакторинга одинаково (кроме явно оговорённых изменений в Фазе 4).
