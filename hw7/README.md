# HW7 — открытие и закрытие тикета по MVVM

Веб-форма на Rust: бэкенд на axum поверх event-sourcing ядра из HW6, клиент на Leptos (wasm).

## Запуск

```sh
rustup target add wasm32-unknown-unknown   # один раз
cargo install trunk                         # один раз

cd client && trunk build && cd ..
cargo run -p server                         # http://127.0.0.1:3000
```

Для разработки клиента: `cargo run -p server` и в `client/` — `trunk serve`
(http://127.0.0.1:8080, запросы `/api/` проксируются на сервер).

Тесты: `cargo test --workspace`.

## Структура

| Crate | Что внутри |
|---|---|
| `ticketing` | Домен и сервис приложения из HW6 + открытие закрытого тикета |
| `api` | Контракт HTTP: DTO и пути, общие для сервера и клиента |
| `server` | REST-адаптер над `TicketService`, демо-данные, раздача собранного клиента |
| `client` | Model, ViewModel и View |

## MVVM в клиенте

- **Model** — `client/src/model`: данные — DTO из `api` (`TicketDto`, `UserDto`, `TenantDto`),
  правила — трейт `TicketRules` с `validate_open` / `validate_close` / `validate_assign`
  для `TicketDto`, а также `TicketApiService` —
  сервис, который отправляет команды на сервер через `HttpClient` (в браузере это `fetch`,
  в тестах — `MockHttpClient`).
- **ViewModel** — `client/src/view_model.rs`: `TicketViewModel` хранит состояние в реактивных
  сигналах, `can_open` / `can_close` / `can_assign` играют роль `CanExecute`, асинхронные
  `open_ticket` / `close_ticket` / `assign_agent` — роль команд. Пока запрос в полёте, `is_busy`
  блокирует остальные команды, интерфейс не замирает.
- **View** — `client/src/view.rs`: разметка, в которой кнопки и поля только привязаны к
  свойствам и командам модели представления.

## Адаптация задания под Rust

- «Выбрасывается исключение» → функция возвращает `Err` с конкретным вариантом ошибки.
- `OpenTicket` → открытие закрытого тикета заново (`TicketCommand::Reopen`, событие
  `TicketReopened`): тикет снова `Open`, без агента, с новым отсчётом неактивности. Открыть
  уже открытый тикет нельзя (`AlreadyOpen`), закрыть закрытый — тоже (`Closed`).
- Скидка тенанта: больше 5 открытых тикетов — 10%, иначе 0%. После открытия или закрытия
  тикета клиент пересчитывает скидку его тенанта.
- Проверки есть на обеих сторонах: клиент не отправляет заведомо недопустимую команду,
  сервер всё равно проверяет её в агрегате и отвечает 404 / 409 / 422 с `{"error": ...}`.

## Где тесты по пунктам задания

| Пункт | Тесты |
|---|---|
| 9. `CloseTicket` / `OpenTicket` | `ticketing/src/domain/ticket/mod.rs`: `close_moves_open_ticket_to_closed`, `closing_closed_ticket_is_rejected`, `reopen_moves_closed_ticket_to_open`, `reopening_open_ticket_is_rejected`; клиентская модель — `client/src/model/mod.rs` |
| 10. Агент и скидка | `only_agents_can_be_assigned` (домен), `only_agent_can_be_assigned` (клиент), `discount.rs`: `no_discount_up_to_threshold`, `discount_applies_above_threshold` |
| 11. Бэкенд через mock | `client/src/model/ticket_api.rs`: успешный и неуспешный ответ для закрытия и открытия, ошибки транспорта и разбора ответа |
| ViewModel | `client/src/view_model.rs`: доступность команд, синхронизация после команд, показ ошибок |
| HTTP API | `server/src/lib.rs`: роуты через `tower::ServiceExt::oneshot` |
