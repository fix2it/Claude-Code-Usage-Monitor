# agy: два пула квоты — передача работы (2026-10-17)

## Задача
Usage Monitor (C:\3_Work\VibeCode\Usage_Monitor, Rust tray) должен показывать квоту agy как ДВА пула:
1. Gemini (большая квота);
2. внешние модели «Claude and GPT models» (claude-sonnet-4-6, claude-opus-4-6-thinking, gpt-oss-120b-medium).
Сейчас в кэше лежит только один пул под ключом `antigravity`, и это внешний пул, а не Gemini.

## Что уже выяснено в коде
- src/poller/antigravity.rs, `antigravity_usage_from_summary` (~стр. 520): возвращает первую группу, в которой
  есть «gemini» (в displayName/description/bucketId/bucket displayName) И есть бакеты с window "5h" или "weekly".
  Если такой нет, берётся первая группа, у которой есть такие окна (fallback).
- `antigravity_usage_from_summary_group`: понимает только окна "5h" (→ session) и "weekly" (→ weekly).
  Гипотеза: у группы Gemini другие имена окон (или их нет), поэтому она не подходит и срабатывает fallback на внешнюю группу. На реальном ответе это НЕ проверено.
- Запрос: POST {base}/v1internal:retrieveUserQuotaSummary с телом {"project": ...}; project берётся из loadCodeAssist.
  Базовые адреса перечислены в ANTIGRAVITY_ENDPOINTS.
- Провайдеры — это enum ProviderId (src/providers.rs, 7 штук, cache_key, native_menu_command_id 60-66).
  Чтобы получить второй виджет, скорее всего, нужен новый ProviderId (например AntigravityExternal), а также правки в
  app_settings.rs (show_*), context_menu.rs, localization, ui/theme.rs (иконка), theme_engine (runtime, theme_limits, theme_expression),
  studio_helper_panels.rs, poller.rs (регистрация) и тестах. Вариант проще: один poll возвращает обе группы, а второй провайдер берёт их из общего результата.
- Тесты для выбора группы: src/poller/tests.rs (`antigravity_summary_prefers_gemini_group`, ~стр. 353).

## Требования к ключам кэша
Кэш: %APPDATA%\ClaudeCodeUsageMonitor\usage-cache.json. Его читает C:\Users\aleks\.claude\skills\handler\scripts\limits.py
(ключи claude_code, codex, antigravity; session/weekly {percentage = ИСПОЛЬЗОВАНО %, resets_at.secs_since_epoch}).
План: новые ключи `antigravity_gemini` и `antigravity_external`; `antigravity` пока остаётся = внешний пул, чтобы limits.py не сломался.
Если у Gemini нет окна, показывать «no limit data», а не скрывать и не выдумывать значение.

## Текущее состояние репозитория
- Не закоммичено: в конец src/poller/antigravity.rs добавлен модуль `live_probe` с тестом `#[ignore]` `live_quota_summary_shape`.
  Он печатает JSON quota-summary (без токенов и project id). Запуск:
  `cargo test live_quota_summary_shape -- --ignored --nocapture`
  Ни разу не запускался (сборку прервали). Перед коммитом зонд удалить или оставить намеренно.
- Ограничения: учётные данные не менять, без коммитов и push без согласия владельца; прежде чем перезапускать работающий монитор, спросить владельца.

## Следующие шаги
1. Запустить зонд и посмотреть, как выглядит группа Gemini (окна, имена).
2. Разбирать обе группы и писать оба ключа (плюс совместимый `antigravity`).
3. Сделать два виджета: «agy Gemini» и «agy Claude+GPT».
4. Тесты на выбор групп; `cargo build` и `cargo test`.

## Итоги ревью 2026-10-18 (loop-code-review, уровни 1-4, Codex gpt-6.1-sol medium, 5 раундов: 6 → 7 → 7 → 7.5 → 8)
- Ключи кэша: `antigravity_gemini`, `antigravity_external`, `antigravity` (= внешний пул; если внешнего пула нет, ключ НЕ пишется, limits.py показывает unknown).
  У отсутствующего окна в кэше `percentage: null`.
- Внешний пул берётся только из retrieveUserQuotaSummary. Квоты отдельных моделей (fetchAvailableModels) не указывают окно, поэтому используются только для Gemini и только когда summary недоступен.
- Безымянная группа назначается Gemini, только если она единственная; бакеты `3p-*` относятся к внешнему пулу.
- Меню: 62 = весь Antigravity (как раньше), 68 = Gemini, 67 = Claude/GPT.
- Последняя правка (Gemini 5h больше не дополняется квотой модели) отдельным ревью не проверялась.
- Тесты studio_app `diagnostics_page_has_logging_controls_and_menu_version` и `version_text_and_trailing_icon_share_one_update_button` падают и на чистом HEAD.

## Ветки для запросов на слияние (2026-10-18, локально, не запушены)
- `fix/duration-short-rounding` (от main) — округление `duration_short`. Сборка: builds/pr1-duration-rounding.exe.
- `feat/agy-quota-request` (от main) — запрос данных agy, только пул Gemini. Сборка: builds/pr2-agy-quota-request.exe.
  Внимание: в этой ветке одной ключ `antigravity` = Gemini (как задумано в upstream); limits.py ждёт там внешний пул.
- `feat/agy-second-pool` (поверх feat/agy-quota-request) — второй пул, ключи кэша, настройки, меню, Compact. Сборка: builds/pr3-agy-second-pool.exe.
- `wip/agy-two-pools-full` — снимок всей рабочей копии до разделения (включая тест-зонд live_probe).
- Изменения для Classic отменены; тест-зонд live_probe в ветки PR не попал.
- Рабочие копии веток: scratchpad\wt\{time,request,pool} (git worktree).

## Claude: нет данных (2026-10-18)
В usage-cache.json у аккаунта Claude стоит `error: {http_status: 429}`, `usage: null`. Код опроса Claude в этой задаче не менялся.
Прямой запрос к /api/oauth/usage в тот же момент вернул 200.
Гипотеза (не подтверждена логом): после реального 429 с Retry-After монитор держит паузу в памяти (до 24 ч, src/poller/retry_after.rs) и отдаёт 429, не обращаясь к серверу. Перезапуск монитора сбрасывает паузу.

## Дополнительные раунды 6-10 (2026-10-18), итог 9.5/10 — принято
- Первый успешный summary авторитетен для обоих пулов, ответы серверов не склеиваются; квоты моделей используются только при полном отсутствии summary и только для Gemini.
- Пустой summary (`groups: []` или группы без окон) = «нет данных».
- Общая привязка `antigravity.*` (Classic, classic-vertical, minecraft, пользовательские темы) следует выбранному пулу; при обоих включённых чередуется по 8-секундному таймеру; `antigravity.pool` = "Gemini" / "Claude/GPT".
- Пул без окон: `antigravity.available = 0` (иконка-заглушка), текст «--».
- Иконка трея обновляется при смене пула.
- Оставшиеся пограничные (уровень 5): в compact пулы различаются цветом, без подписи; округление обратного отсчёта может обновлять текст позже границы.
