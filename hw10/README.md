# HW9 — саги, хореография и дедлайны

Жизненный цикл тикета в системе из HW6–HW8: оркестратор с дедлайнами напоминает о простое
и закрывает неактивные тикеты, а уведомления, скидки и read-модель независимо реагируют на
события. Всё на Rust: ядро и сервер на tokio + axum, клиент на Leptos (wasm).

Устройство системы — абстрактное ядро, сага, полная схема команд и событий, протокол клиента
и сервера — описано в [ARCHITECTURE.md](ARCHITECTURE.md). Здесь — про реализацию.

## Запуск

```sh
rustup target add wasm32-unknown-unknown   # один раз
cargo install trunk                         # один раз

cd client && trunk build && cd ..
cargo run -p server                         # http://127.0.0.1:3000, система пуста
cargo run -p server -- --seed               # с демо-данными
cargo run -p server -- --time-scale 8640    # часы идут быстрее: сутки за 10 секунд
cargo run -p server -- --port 8000          # другой порт; --port 0 — любой свободный
```

Разработка клиента: сервер запущен отдельно, в `client/` — `trunk serve`
(http://127.0.0.1:8080, запросы `/api/` проксируются на порт 3000). Если сервер на другом
порту: `trunk serve --proxy-backend http://127.0.0.1:8000/api/`.

Тесты: `cargo test --workspace`.

### Что посмотреть: дедлайны

При `--time-scale 8640` сутки длятся 10 секунд: напоминания приходят через 30 и 90 секунд,
автозакрытие — через 100. Домен и сага об ускорении не знают: для них проходят обычные 10 дней.

1. Запустите `cargo run -p server -- --time-scale 8640` и откройте http://127.0.0.1:3000.
   Система пуста, подсказка под таблицей говорит, что делать дальше.
2. В панели «Create» заведите тенанта, агента, администратора и два тикета. На карточке
   тикета виден срок автозакрытия — через 10 дней.
3. В панели уведомлений выберите администратора: там «new ticket needs an agent».
4. Назначьте агента на второй тикет. У него сдвинутся сроки, а у агента появится
   «assigned to you».
5. Через 30 секунд после создания первого тикета администратор получит «reminder: stale»,
   а на карточке «Reminders sent» станет 1. По второму тикету такое же напоминание придёт
   агенту через 30 секунд после назначения.
6. Через 90 секунд придёт «reminder: closing soon», а через 100 первый тикет закроется сам.
   На карточке появится причина «inactivity», а администратор получит «closed: inactivity».
7. Откройте закрытый тикет заново: процесс начнётся сначала, счётчик напоминаний обнулится.

### Что посмотреть: хореография и long polling

1. Запустите `cargo run -p server -- --seed` и откройте http://127.0.0.1:3000 в двух вкладках.
   В шапке — `live · vN`: версия, до которой клиент получил изменения.
2. Закройте тикет Acme в одной вкладке. Её кнопки блокируются, пока сервер не пришлёт
   результат; во второй вкладке статус тикета, счётчик открытых тикетов и скидка Acme
   (10% → 0%, её пересчитывает сервер) меняются почти сразу.
3. В панели «Queries» выберите запрос — например, активные тикеты Alice или закрытые
   за последние 7 дней. Результат обновляется сам, когда меняются данные.
4. Остановите сервер — индикатор станет `offline · reconnecting…`; запустите снова —
   клиент переподключится и получит новое состояние целиком.

Уведомления сервер также пишет в stdout — это его адаптер доставки `LogNotifier`:

```
notification to agent user-1: ticket-2 reminder: stale at t=1793800958s
```

## Структура

| Крейт | Что внутри | Главные файлы |
|---|---|---|
| `framework` | Абстрактное ядро без знания о тикетах: event sourcing, CQRS, саги с дедлайнами, in-memory хранилища | `event_sourcing/{lifecycle,repository}.rs`, `cqrs/{command_bus,event_source,projection,query,policy,runner}.rs`, `saga/{mod,effects,deadlines,manager}.rs` |
| `ticketing` | Домен (агрегаты `Ticket`, `Tenant`, `User`, их команды и события) и слой приложения: сервис, сага, политики, часы | `domain/*/events.rs`, `application/{ticket_service,ticket_lifecycle,notifications,discount_policy,clock}.rs` |
| `command` | Командная сторона: актор — единственный писатель, глобальные версии, подписка на события, цикл саги | `actor.rs`, `handle.rs`, `lifecycle.rs` |
| `query` | Запросная сторона: модели представления, запросы, read-модель с индексами | `views.rs`, `queries.rs`, `state.rs`, `read_model.rs`, `tests/pipeline.rs` |
| `api` | HTTP-контракт: DTO и пути, общие для сервера и клиента | `lib.rs` |
| `server` | Сборка системы и HTTP: команды, запросы, long polling, доставка уведомлений, демо-данные | `lib.rs`, `commands.rs`, `queries.rs`, `notifier.rs`, `seed.rs`, `main.rs` |
| `client` | Веб-клиент по MVVM | `model/{replica,ticket_api}.rs`, `view_model.rs`, `view.rs` |

Клиент устроен по MVVM, как в HW7:

- **Model** — DTO из `api`, правила допустимости команд `TicketRules`, сборка запросов на создание
  (`new_tenant`, `new_user`, `new_ticket`), `Replica` — копия read-модели сервера,
  `TicketApiService` — команды, long polling и запросы через `HttpClient` (в браузере `fetch`,
  в тестах `MockHttpClient`).
- **ViewModel** — `TicketViewModel`: реактивные сигналы, формы `CreationForms`, `can_open` /
  `can_close` / `can_assign` / `can_create_*` (CanExecute), асинхронные команды, цикл
  синхронизации `sync_forever`, входящие выбранного пользователя, панель запросов.
- **View** — разметка, привязанная к свойствам и командам модели представления.

## Адаптация задания

- **«Классы» агрегатов, команд и событий** → структуры и перечисления Rust. Агрегаты и большая
  часть команд остались из HW6–HW8; новые — `SendReminderCommand` и `ReminderSentEvent`.
- **Оркестратор** → `TicketLifecycleSaga`: чистый автомат над абстракцией `Saga` из `framework`,
  а время, очередь и отправку команд берёт на себя `run_ticket_lifecycle`.
- **Удаление дедлайна** → `DeadlineQueue::cancel_all`: дедлайны снимаются сразу, а не проверяются
  при наступлении срока. См. [ARCHITECTURE.md](ARCHITECTURE.md#51-оркестратор-ticketlifecyclesaga).
- **Классы, реагирующие на события** → `NotificationPolicy`, `DiscountPolicy`, проекция `ReadModel`.
- **Уведомление агента** → порт `Notifier` с адаптером `LogNotifier` и входящие в read-модели.

Отличия от текста задания — решения, принятые при обсуждении:

| В задании | В коде | Почему |
|---|---|---|
| Дедлайн 10 дней после создания | 10 дней после последней активности: создания, назначения агента, повторного открытия | Иначе тикет, только что взятый в работу, мог бы закрыться на следующий день. Срок 7 дней из HW8 заменён на 10. |
| Напоминание, если тикет открыт более 3 дней, и перед истечением срока | Два напоминания: на 3-й и на 9-й день | Оба условия задания: простой и скорое закрытие. Каждое приходит один раз. |
| Уведомить агента | Если агента нет, напоминание идёт администраторам | Иначе о тикете без агента никто бы не узнал. |
| Оркестратор «активирует» `TicketCreatedEvent` | Сага стартует, получив `TicketCreated` (как `@StartSaga` в Axon) | Событие пишет агрегат: запись и старт процесса разделены. Создание работает и без запущенной саги, а она догонит журнал. |
| При истечении дедлайна «активируйте событие» | Сага отправляет `CloseTicket`, а `TicketClosed` выпускает агрегат | События — решения агрегата: он проверяет, что срок действительно истёк. |
| — | Демо-данные только по `--seed`, создание — через API и формы клиента | Систему можно пройти от начала, без данных, зашитых в сервер. |
| — | `--time-scale` | Сроки в днях иначе не увидеть на запущенной системе. |

## Пункты задания и где они в коде

| Пункт | Код | Тесты |
|---|---|---|
| 1. Агрегаты и команды | `ticketing/src/domain/{ticket,tenant,user}`, `ticketing/src/application/commands.rs` | `ticketing/src/domain/ticket/mod.rs`: `reminder_is_addressed_to_assigned_agent`, `reminder_without_agent_is_addressed_to_admins`, `reminder_before_its_time_is_rejected`, `closed_ticket_gets_no_reminder`, `reminder_is_not_repeated_and_goes_only_forward`, `active_ticket_cannot_be_closed_for_inactivity` |
| 2. Оркестратор | `ticketing/src/application/ticket_lifecycle.rs`, `framework/src/saga`, `command/src/lifecycle.rs` | `creation_schedules_reminders_and_auto_close`, `closing_before_deadline_finishes_process_and_removes_deadlines`, `assignment_moves_all_deadlines`, `reopening_starts_a_new_process`, `ticket_closed_before_deadline_gets_nothing` |
| 2. Напоминания | `Ticket::send_reminder`, `SendReminderCommand`, `ReminderSentEvent` | `deadlines_fire_in_their_time_and_not_earlier`, `unassigned_ticket_is_reminded_to_admins_and_closed`, `assignment_moves_deadlines_and_reminder_goes_to_agent` |
| 3. Хореография | `ticketing/src/application/{notifications,discount_policy}.rs`, `framework/src/cqrs/runner.rs` | `assignment_notifies_the_agent`, `reassignment_also_tells_the_previous_agent`, `reminder_goes_to_the_agent_or_to_admins`, `closing_tells_whoever_is_responsible`, `events_changing_open_tickets_recalculate_discount` |
| 3. Обновление запросной модели | `query/src/state.rs`, `query/src/read_model.rs` | `closing_records_time_and_frees_agent_and_tenant`, `reminder_is_counted_and_shows_in_changes`, `agent_sees_only_own_notifications_in_order`, `notifications_appear_in_changes_once` |
| 4. Дедлайны и автозакрытие | `framework/src/saga/deadlines.rs`, `TicketDeadline`, `run_ticket_lifecycle` | `untouched_ticket_is_reminded_and_closed_on_schedule`, `aggregate_accepts_commands_exactly_on_saga_deadlines`, `stale_auto_close_is_rejected_by_aggregate_after_activity`, `tickets_are_closed_by_deadline_and_discount_follows` |
| 5. Тест: дедлайн создаётся и удаляется при закрытии | — | `creation_schedules_reminders_and_auto_close`, `closing_before_deadline_finishes_process_and_removes_deadlines`, `finishing_removes_instance_and_all_its_deadlines` |
| 5. Тест: назначение уведомляет и обновляет статус | — | `query/tests/pipeline.rs`: `assignment_notifies_agent_and_updates_ticket_status` |
| 5. Тест: напоминания и закрытие в срок | — | `query/tests/pipeline.rs`: `saga_reminders_and_auto_close_reach_read_model`, `saga_reminders_reach_agent_or_admins_through_notifications`; `server/src/lib.rs`: `reminders_reach_ticket_and_inbox_over_http`, `accelerated_clock_closes_ticket_in_real_time` |

Сверх задания:

| Что | Код | Тесты |
|---|---|---|
| Лавина дедлайнов: по одному за раз, события вперёд таймера | `SagaManager::fire_next`, `run_ticket_lifecycle` | `deadlines_are_fired_one_at_a_time`, `event_between_deadlines_cancels_the_next_one`, `event_between_due_deadlines_cancels_the_next_one`, `overdue_deadlines_run_in_order_without_rejections` |
| Счётчик открытых тикетов за O(1) | `ticketing/src/application/open_tickets.rs` | тесты в том же файле |
| Ускоренные часы | `ScaledClock`, `Config::for_time_scale`, `--time-scale` | `scaled_clock_runs_scale_times_faster`, `deadline_tick_shrinks_with_time_scale`, `bad_time_scale_is_reported` |
| Создание через API и клиент | `server/src/commands.rs`, `client/src/view_model.rs` (`CreationForms`) | `empty_system_is_filled_through_commands`, `creating_with_bad_data_is_rejected`, `creating_tenant_clears_form_and_waits_for_changes`, `rejected_creation_keeps_input_and_shows_error_in_its_own_place` |
| Панель уведомлений и сроки в клиенте | `client/src/model/replica.rs`, `client/src/view.rs` | `inbox_holds_only_what_is_addressed_to_the_user`, `notifications_belong_to_chosen_user_and_arrive_by_polling`, `long_polling_brings_new_notifications_once`, `ticket_shows_its_deadline_and_how_it_was_closed` |

Из HW8 остались: командная сторона как актор, query side и long polling.

## Тесты

| Крейт | Тестов | Что проверяют |
|---|---|---|
| `framework` | 37 | `Lifecycle` для `Option<T>`, репозиторий и снапшоты, in-memory хранилище, `run_projection`, `run_policy`, очередь дедлайнов, `SagaManager` |
| `ticketing` | 105 | агрегаты, сервис приложения, outbox, `OpenTickets`, сага с настоящими агрегатами, политики скидок и уведомлений, часы |
| `command` | 15 | версии, подписка, отказы, параллельные отправители, цикл саги на остановленном времени tokio |
| `query` | 29 + 6 | проекция и запросы; интеграционные — весь конвейер актор → сага и политики → проекция → запросы |
| `server` | 22 + 7 | HTTP API, long polling, сроки в настоящем времени с `ScaledClock`; разбор аргументов командной строки |
| `api` | 1 | текст уведомлений |
| `client` | 51 | правила команд, формы создания, `Replica`, входящие, `Backoff`, `TicketApiService` на заглушке HTTP, ViewModel, форматирование дат |

Разметку клиента тесты в репозитории не покрывают: её проверял разовый прогон сценария
в headless Chromium, сценарий в репозиторий не входит.
