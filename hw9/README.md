# HW8 — Query Side по CQRS

Запросная сторона для системы тикетов и тенантов из HW6–HW7: read-модель, которую строят
обработчики событий командной стороны, API запросов к ней и живое обновление клиентов
через long polling. Всё на Rust: ядро и сервер на tokio + axum, клиент на Leptos (wasm).

Устройство системы — абстрактное ядро, полная схема команд и событий, протокол клиента
и сервера — описано в [ARCHITECTURE.md](ARCHITECTURE.md). Здесь — про реализацию.

## Запуск

```sh
rustup target add wasm32-unknown-unknown   # один раз
cargo install trunk                         # один раз

cd client && trunk build && cd ..
cargo run -p server                         # http://127.0.0.1:3000
cargo run -p server -- --port 8000          # другой порт; --port 0 — любой свободный
```

Разработка клиента: сервер запущен отдельно, в `client/` — `trunk serve`
(http://127.0.0.1:8080, запросы `/api/` проксируются на порт 3000). Если сервер на другом
порту: `trunk serve --proxy-backend http://127.0.0.1:8000/api/`.

Тесты: `cargo test --workspace`.

### Что посмотреть

1. Откройте http://127.0.0.1:3000 в двух вкладках. В шапке — `live · vN`: версия, до
   которой клиент получил изменения.
2. Закройте тикет Acme в одной вкладке. Её кнопки блокируются, пока сервер не пришлёт
   результат; во второй вкладке статус тикета, счётчик открытых тикетов и скидка Acme
   (10% → 0%, её пересчитывает сервер) меняются почти сразу.
3. В панели «Queries» выберите запрос — например, активные тикеты Alice или закрытые
   за последние 7 дней. Результат обновляется сам, когда меняются данные.
4. Остановите сервер — индикатор станет `offline · reconnecting…`; запустите снова —
   клиент переподключится и получит новое состояние целиком.

## Структура

| Крейт | Что внутри | Главные файлы |
|---|---|---|
| `framework` | Абстрактное ядро без знания о тикетах: event sourcing, CQRS, in-memory хранилища | `event_sourcing/{lifecycle,repository}.rs`, `cqrs/{command_bus,event_source,projection,query,policy,runner}.rs` |
| `ticketing` | Домен (агрегаты `Ticket`, `Tenant`, `User`, их команды и события) и сервис приложения | `domain/*/events.rs`, `application/ticket_service.rs`, `application/discount_policy.rs` |
| `command` | Командная сторона: актор — единственный писатель, глобальные версии, подписка на события, расписание | `actor.rs`, `handle.rs`, `schedule.rs` |
| `query` | Запросная сторона: модели представления, запросы, read-модель с индексами | `views.rs`, `queries.rs`, `state.rs`, `read_model.rs`, `tests/pipeline.rs` |
| `api` | HTTP-контракт: DTO и пути, общие для сервера и клиента | `lib.rs` |
| `server` | Сборка системы и HTTP: команды, запросы, long polling, демо-данные | `lib.rs`, `commands.rs`, `queries.rs`, `seed.rs`, `main.rs` |
| `client` | Веб-клиент по MVVM | `model/{replica,ticket_api}.rs`, `view_model.rs`, `view.rs` |

Клиент устроен по MVVM, как в HW7:

- **Model** — DTO из `api`, правила допустимости команд `TicketRules`, `Replica` — копия
  read-модели сервера, `TicketApiService` — команды, long polling и запросы через `HttpClient`
  (в браузере `fetch`, в тестах `MockHttpClient`).
- **ViewModel** — `TicketViewModel`: реактивные сигналы, `can_open` / `can_close` / `can_assign`
  (CanExecute), асинхронные команды, цикл синхронизации `sync_forever`, панель запросов.
- **View** — разметка, привязанная к свойствам и командам модели представления.

## Адаптация задания под Rust

- **«Класс `TicketView` / `TenantView`»** → структуры в `query/src/views.rs`. В `TicketView`
  кроме полей из задания есть `created_at`, `closed_at` и `version`; в `TenantView` —
  `discount_rate` и `open_tickets`. Добавлен `UserView` — для списка агентов и их имён.
- **`TicketCreatedEvent`, `AgentAssignedEvent`, `TicketClosedEvent`, `DiscountAppliedEvent`**
  — события из HW6 (`ticketing/src/domain/*/events.rs`), плюс `TicketReopenedEvent`,
  `TenantRegisteredEvent`, `UserRegisteredEvent`. В `TicketClosedEvent` добавлен `tenant_id`:
  он нужен политике скидок и проекциям.
- **«Черновики тикетов тенанта»** — черновиков в домене нет. Ближайший аналог —
  `UnassignedTicketsOfTenant`: тикет заведён, но работа по нему не начата (открыт, агента нет).
- **Хранилище, оптимизированное под чтение** — in-memory: денормализованные записи
  в `BTreeMap` и индексы под запросы (по агенту, по тенанту, по времени закрытия,
  по версии изменения).
- **Асинхронная обработка событий** — проекция работает в отдельной задаче tokio
  и получает события из подписки на актор; команда не ждёт обновления read-модели.
  Ядро event sourcing тоже асинхронное: трейты хранилищ возвращают `Send`-future.
- **Скидка** пересчитывается на сервере политикой `DiscountPolicy`, а не клиентом:
  больше 5 открытых тикетов — 10%, иначе 0%.
- **«Выбрасывается исключение»** → `Err` с конкретным вариантом ошибки; на HTTP —
  404 / 409 / 422 с `{"error": ...}`.

## Пункты задания и где они в коде

| Пункт | Код | Тесты |
|---|---|---|
| 1. Модели запросов `TicketView`, `TenantView` | `query/src/views.rs` | `query/src/state.rs`: `registrations_create_tenant_and_user_views`, `created_ticket_is_open_and_counts_for_tenant` |
| 2. Обработчики событий | `query/src/state.rs` (`State::apply`), `query/src/read_model.rs` (`Projection`) | `state.rs`: по тесту на каждое событие — `assignment_sets_agent_and_moves_ticket_between_agents`, `closing_records_time_and_frees_agent_and_tenant`, `reopened_ticket_is_open_unassigned_and_counts_again`, `discount_applied_updates_tenant`; надёжность — `redelivered_event_is_ignored`, `gap_in_versions_is_an_error`, `event_for_unknown_record_changes_nothing` |
| 3. Хранилище под чтение | `query/src/state.rs`: записи и индексы | `active_tickets_of_agent_skip_closed_and_others`, `list_tickets_filters_by_status_and_tenant`, `changes_since_return_each_record_once_in_latest_state` |
| 4. API запросов | `query/src/queries.rs`, `server/src/queries.rs` | `server/src/lib.rs`: `seeded_state_is_served_from_read_model`, `read_side_queries`, `unknown_ticket_and_tenant_are_not_found` |
| 5. Сложные запросы: активные тикеты агента, закрытые за 7 дней | `ActiveTicketsOfAgent`, `TicketsClosedSince`, `UnassignedTicketsOfTenant` | `state.rs`: `closed_since_includes_boundary_and_orders_by_closing_time`, `unassigned_tickets_are_open_without_agent_of_that_tenant`; `server/src/lib.rs`: `closed_within_days_is_counted_from_now` |
| 6. Тесты обработки событий | — | `query/src/state.rs`, `query/src/read_model.rs` |
| 6. Тесты API запросов | — | `server/src/lib.rs` (через `tower::ServiceExt::oneshot`), `client/src/model/ticket_api.rs` |
| Доп.: асинхронная обработка событий | `framework/src/cqrs/runner.rs` (`run_projection`), задачи в `server/src/lib.rs` | `query/tests/pipeline.rs`: `read_model_follows_commands`, `late_projection_catches_up_from_history`, `closing_inactive_tickets_reaches_read_model`; `read_model.rs`: `wait_until_wakes_up_when_version_is_projected` |

Сверх задания:

| Что | Код | Тесты |
|---|---|---|
| Командная сторона как актор с глобальными версиями | `command/src/actor.rs` | `every_event_gets_next_global_version`, `concurrent_senders_are_serialized`, `subscriber_gets_history_after_version_then_live_events` |
| Политика скидок | `ticketing/src/application/discount_policy.rs`, `run_policy` | `discount_policy.rs`, `runner.rs`, `pipeline.rs`: `discount_policy_follows_open_tickets`, `server`: `discount_is_recalculated_by_policy` |
| Автозакрытие неактивных тикетов по расписанию | `command/src/schedule.rs` | `inactive_tickets_are_closed_on_schedule` (на остановленном времени tokio) |
| Long polling и синхронизация клиентов | `server/src/queries.rs` (`changes`), `client/src/model/replica.rs`, `client/src/view_model.rs` | `server`: `long_poll_answers_as_soon_as_something_changes`, `long_poll_times_out_with_empty_changes`, `cursor_from_another_server_run_gets_everything`; клиент: `command_keeps_model_busy_until_its_version_arrives`, `server_restart_releases_pending_command`, `query_results_follow_changes` |
| Абстрактное ядро | `framework` | `lifecycle.rs`, `repository.rs`, `runner.rs`, `in_memory/event_store.rs` |

## Тесты

| Крейт | Тестов | Что проверяют |
|---|---|---|
| `framework` | 18 | `Lifecycle` для `Option<T>`, репозиторий и снапшоты, in-memory хранилище, `run_projection`, `run_policy` |
| `ticketing` | 63 | агрегаты, сервис приложения, outbox, `OpenTickets`, политика скидок |
| `command` | 9 | версии, подписка, отказы, параллельные отправители, расписание |
| `query` | 18 + 4 | проекция и запросы; интеграционные — весь конвейер актор → проекция → запросы и политика |
| `server` | 12 + 4 | HTTP API и long polling; разбор аргументов командной строки |
| `client` | 36 | правила команд, `Replica`, `Backoff`, `TicketApiService` на заглушке HTTP, ViewModel, форматирование дат |
