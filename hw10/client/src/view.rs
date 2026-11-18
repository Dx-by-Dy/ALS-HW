//! View: разметка формы. Своей логики здесь нет — только привязки к свойствам и командам
//! `TicketViewModel` и форматирование значений для показа.

use std::time::Duration;

use leptos::{ev::SubmitEvent, prelude::*, task::spawn_local};

use api::{NotificationDto, Role, TicketDto, TicketStatus};

use crate::model::http::BrowserHttpClient;
use crate::model::ticket_api::TicketQuery;
use crate::view_model::TicketViewModel;

type Vm = TicketViewModel<BrowserHttpClient>;

/// Сколько дней показывает запрос закрытых тикетов, пока пользователь не ввёл своё.
const DEFAULT_CLOSED_DAYS: u64 = 7;

const ROLES: [Role; 3] = [Role::Agent, Role::Admin, Role::Customer];

#[component]
pub fn TicketView(vm: Vm) -> impl IntoView {
    spawn_local(async move { vm.sync_forever(sleep).await });
    view! {
        <main class="app">
            <header class="topbar">
                <h1>"Support tickets"</h1>
                <span class="live" class:offline=move || !vm.is_connected()>
                    {move || match (vm.is_connected(), vm.version()) {
                        (true, Some(version)) => format!("live · v{version}"),
                        (false, Some(_)) => "offline · reconnecting…".to_owned(),
                        (_, None) => "connecting…".to_owned(),
                    }}
                </span>
                <span class="sync" class:visible=move || vm.is_busy()>
                    "Waiting for the server…"
                </span>
            </header>
            <TenantBar vm />
            <CreationPanel vm />
            <div class="layout">
                <TicketList vm />
                <div class="side">
                    <TicketDetails vm />
                    <NotificationPanel vm />
                </div>
            </div>
            <QueryPanel vm />
        </main>
    }
}

#[component]
fn TenantBar(vm: Vm) -> impl IntoView {
    view! {
        <section class="tenants">
            {move || {
                vm.tenants()
                    .into_iter()
                    .map(|tenant| {
                        let discounted = tenant.discount_percent > 0;
                        view! {
                            <div class="tenant" class:discounted=discounted>
                                <span class="tenant-name">{tenant.tenant_name}</span>
                                <span>{format!("{} open tickets", tenant.open_tickets)}</span>
                                <span class="discount">
                                    {format!("discount {}%", tenant.discount_percent)}
                                </span>
                            </div>
                        }
                    })
                    .collect_view()
            }}
        </section>
    }
}

/// Формы создания: сервер может стартовать пустым, и всё заводится отсюда.
#[component]
fn CreationPanel(vm: Vm) -> impl IntoView {
    let forms = vm.forms;
    view! {
        <details class="panel creation" open>
            <summary>"Create"</summary>
            <div class="creation-forms">
                <form on:submit=move |ev| submit(ev, async move { vm.create_tenant().await })>
                    <h3>"Tenant"</h3>
                    <label>"Name" <input bind:value=forms.tenant_name /></label>
                    <button type="submit" disabled=move || !vm.can_create_tenant()>
                        "Create tenant"
                    </button>
                </form>
                <form on:submit=move |ev| submit(ev, async move { vm.create_user().await })>
                    <h3>"User"</h3>
                    <label>"Name" <input bind:value=forms.user_name /></label>
                    <label>
                        "Role"
                        <select on:change=move |ev| {
                            let value = event_target_value(&ev);
                            if let Some(role) = ROLES.into_iter().find(|r| r.to_string() == value) {
                                forms.user_role.set(role);
                            }
                        }>
                            {ROLES
                                .into_iter()
                                .map(|role| {
                                    view! {
                                        <option selected=move || forms.user_role.get() == role>
                                            {role.to_string()}
                                        </option>
                                    }
                                })
                                .collect_view()}
                        </select>
                    </label>
                    <button type="submit" disabled=move || !vm.can_create_user()>
                        "Create user"
                    </button>
                </form>
                <form on:submit=move |ev| submit(ev, async move { vm.create_ticket().await })>
                    <h3>"Ticket"</h3>
                    <label>
                        "Tenant"
                        <select on:change=move |ev| {
                            forms.ticket_tenant.set(event_target_value(&ev).parse().ok());
                        }>
                            <option value="">"Choose a tenant"</option>
                            {move || {
                                vm.tenants()
                                    .into_iter()
                                    .map(|tenant| {
                                        let id = tenant.tenant_id;
                                        view! {
                                            <option
                                                value=id.to_string()
                                                selected=move || forms.ticket_tenant.get() == Some(id)
                                            >
                                                {tenant.tenant_name}
                                            </option>
                                        }
                                    })
                                    .collect_view()
                            }}
                        </select>
                    </label>
                    <label>"Category" <input bind:value=forms.ticket_category /></label>
                    <label>"Description" <input bind:value=forms.ticket_description /></label>
                    <button type="submit" disabled=move || !vm.can_create_ticket()>
                        "Create ticket"
                    </button>
                </form>
            </div>
            <p class="error" role="alert">{move || vm.creation_error()}</p>
        </details>
    }
}

/// Входящие уведомления выбранного пользователя. Выбор — не вход в систему: авторизации нет.
#[component]
fn NotificationPanel(vm: Vm) -> impl IntoView {
    view! {
        <section class="panel notifications">
            <div class="assign">
                <label for="viewer">"Notifications for"</label>
                <select
                    id="viewer"
                    on:change=move |ev| vm.view_as(event_target_value(&ev).parse().ok())
                >
                    <option value="">"Choose a user"</option>
                    {user_options(vm, move || vm.viewer_id())}
                </select>
            </div>
            {move || {
                let notifications = vm.notifications();
                if vm.viewer_id().is_none() {
                    view! { <p class="empty">"Choose a user to see their notifications"</p> }.into_any()
                } else if notifications.is_empty() {
                    view! { <p class="empty">"No notifications"</p> }.into_any()
                } else {
                    view! {
                        <ul>
                            {notifications
                                .into_iter()
                                .map(|notification| notification_row(vm, notification))
                                .collect_view()}
                        </ul>
                    }
                    .into_any()
                }
            }}
        </section>
    }
}

fn notification_row(vm: Vm, notification: NotificationDto) -> impl IntoView {
    let ticket_id = notification.ticket_id;
    view! {
        <li>
            <button type="button" on:click=move |_| vm.select_ticket(ticket_id)>
                <span>{format!("ticket-{ticket_id}: {}", notification.kind)}</span>
                <time>{format_utc(notification.at)}</time>
            </button>
        </li>
    }
}

#[component]
fn TicketList(vm: Vm) -> impl IntoView {
    view! {
        <section class="panel list">
            <table>
                <thead>
                    <tr>
                        <th>"#"</th>
                        <th>"Tenant"</th>
                        <th>"Category"</th>
                        <th>"Description"</th>
                        <th>"Status"</th>
                        <th>"Agent"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        vm.tickets().into_iter().map(|ticket| ticket_row(vm, ticket)).collect_view()
                    }}
                </tbody>
            </table>
            {move || vm.list_hint().map(|hint| view! { <p class="empty list-hint">{hint}</p> })}
        </section>
    }
}

fn ticket_row(vm: Vm, ticket: TicketDto) -> impl IntoView {
    let id = ticket.ticket_id;
    let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
    view! {
        <tr class:selected=move || vm.is_selected(id) on:click=move |_| vm.select_ticket(id)>
            <td>{id}</td>
            <td>{tenant}</td>
            <td>{ticket.category}</td>
            <td>{ticket.description}</td>
            <td>{status_badge(ticket.status)}</td>
            <td>{agent_name(vm, ticket.assigned_agent)}</td>
        </tr>
    }
}

#[component]
fn TicketDetails(vm: Vm) -> impl IntoView {
    view! {
        <section class="panel details">
            {move || match vm.selected() {
                None => view! { <p class="empty">"Select a ticket"</p> }.into_any(),
                Some(ticket) => ticket_card(vm, ticket).into_any(),
            }}
            <div class="actions">
                <button
                    on:click=move |_| spawn_local(async move { vm.open_ticket().await })
                    disabled=move || !vm.can_open()
                >
                    "Open"
                </button>
                <button
                    class="danger"
                    on:click=move |_| spawn_local(async move { vm.close_ticket().await })
                    disabled=move || !vm.can_close()
                >
                    "Close"
                </button>
            </div>
            <div class="assign">
                <label for="assignee">"Assign agent"</label>
                <div class="assign-row">
                    <select
                        id="assignee"
                        on:change=move |ev| {
                            vm.select_user(event_target_value(&ev).parse().ok())
                        }
                    >
                        <option value="">"Choose a user"</option>
                        {user_options(vm, move || vm.selected_user_id())}
                    </select>
                    <button
                        on:click=move |_| spawn_local(async move { vm.assign_agent().await })
                        disabled=move || !vm.can_assign()
                    >
                        "Assign"
                    </button>
                </div>
                <p class="hint">{move || vm.assign_hint()}</p>
            </div>
            <p class="error" role="alert">{move || vm.error()}</p>
        </section>
    }
}

fn ticket_card(vm: Vm, ticket: TicketDto) -> impl IntoView {
    let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
    view! {
        <div class="card">
            <div class="card-title">
                <h2>{format!("ticket-{} · {}", ticket.ticket_id, ticket.category)}</h2>
                {status_badge(ticket.status)}
            </div>
            <p class="description">{ticket.description}</p>
            <dl>
                <dt>"Tenant"</dt>
                <dd>{tenant}</dd>
                <dt>"Agent"</dt>
                <dd>{agent_name(vm, ticket.assigned_agent)}</dd>
                <dt>"Created"</dt>
                <dd>{format_utc(ticket.created_at)}</dd>
                <dt>"Last activity"</dt>
                <dd>{format_utc(ticket.last_activity)}</dd>
                <dt>"Closes by itself"</dt>
                <dd>{optional_time(ticket.closes_at)}</dd>
                <dt>"Reminders sent"</dt>
                <dd>{ticket.reminders_sent}</dd>
                <dt>"Closed"</dt>
                <dd>{optional_time(ticket.closed_at)}</dd>
                <dt>"Close reason"</dt>
                <dd>{ticket.close_reason.map_or_else(|| "—".to_owned(), |reason| reason.to_string())}</dd>
            </dl>
        </div>
    }
}

/// Запросы к read-модели сервера. Результат обновляется сам, когда меняются данные.
#[component]
fn QueryPanel(vm: Vm) -> impl IntoView {
    let run = move |query| spawn_local(async move { vm.run_query(query).await });
    let days = RwSignal::new(DEFAULT_CLOSED_DAYS);
    let is_active = move |pick: fn(TicketQuery) -> bool| vm.query().is_some_and(pick);
    view! {
        <section class="panel queries">
            <h2>"Queries"</h2>
            <div class="query-row" class:active=move || is_active(|q| matches!(q, TicketQuery::ActiveOfAgent(_)))>
                <span>"Active tickets of"</span>
                <select on:change=move |ev| {
                    if let Ok(agent_id) = event_target_value(&ev).parse() {
                        run(TicketQuery::ActiveOfAgent(agent_id));
                    }
                }>
                    <option value="">"Choose an agent"</option>
                    {move || {
                        vm.agents()
                            .into_iter()
                            .map(|agent| {
                                let query = TicketQuery::ActiveOfAgent(agent.user_id);
                                view! {
                                    <option
                                        value=agent.user_id.to_string()
                                        selected=move || vm.query() == Some(query)
                                    >
                                        {agent.user_name}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </div>
            <div class="query-row" class:active=move || is_active(|q| matches!(q, TicketQuery::UnassignedOfTenant(_)))>
                <span>"Unassigned tickets of"</span>
                <select on:change=move |ev| {
                    if let Ok(tenant_id) = event_target_value(&ev).parse() {
                        run(TicketQuery::UnassignedOfTenant(tenant_id));
                    }
                }>
                    <option value="">"Choose a tenant"</option>
                    {move || {
                        vm.tenants()
                            .into_iter()
                            .map(|tenant| {
                                let query = TicketQuery::UnassignedOfTenant(tenant.tenant_id);
                                view! {
                                    <option
                                        value=tenant.tenant_id.to_string()
                                        selected=move || vm.query() == Some(query)
                                    >
                                        {tenant.tenant_name}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </div>
            <div class="query-row" class:active=move || is_active(|q| matches!(q, TicketQuery::ClosedWithin(_)))>
                <span>"Closed in the last"</span>
                <input
                    type="number"
                    min="1"
                    prop:value=move || days.get().to_string()
                    on:change=move |ev| {
                        if let Ok(value) = event_target_value(&ev).parse::<u64>() {
                            days.set(value.max(1));
                            run(TicketQuery::ClosedWithin(days.get_untracked()));
                        }
                    }
                />
                <span>"days"</span>
                <button on:click=move |_| run(TicketQuery::ClosedWithin(days.get_untracked()))>
                    "Show"
                </button>
            </div>
            {move || match (vm.query(), vm.query_error()) {
                (None, _) => {
                    view! { <p class="empty">"Choose a query to ask the read model"</p> }.into_any()
                }
                (Some(_), Some(error)) => view! { <p class="error">{error}</p> }.into_any(),
                (Some(_), None) => query_results(vm, vm.query_results()).into_any(),
            }}
        </section>
    }
}

fn query_results(vm: Vm, tickets: Vec<TicketDto>) -> impl IntoView {
    if tickets.is_empty() {
        return view! { <p class="empty">"No tickets"</p> }.into_any();
    }
    view! {
        <table>
            <thead>
                <tr>
                    <th>"#"</th>
                    <th>"Tenant"</th>
                    <th>"Description"</th>
                    <th>"Status"</th>
                    <th>"Agent"</th>
                    <th>"Closed"</th>
                </tr>
            </thead>
            <tbody>
                {tickets
                    .into_iter()
                    .map(|ticket| {
                        let id = ticket.ticket_id;
                        let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
                        view! {
                            <tr on:click=move |_| vm.select_ticket(id)>
                                <td>{id}</td>
                                <td>{tenant}</td>
                                <td>{ticket.description}</td>
                                <td>{status_badge(ticket.status)}</td>
                                <td>{agent_name(vm, ticket.assigned_agent)}</td>
                                <td>{optional_time(ticket.closed_at)}</td>
                            </tr>
                        }
                    })
                    .collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

fn status_badge(status: TicketStatus) -> impl IntoView {
    let class = match status {
        TicketStatus::Open => "badge open",
        TicketStatus::InProgress => "badge in-progress",
        TicketStatus::Closed => "badge closed",
    };
    view! { <span class=class>{status.to_string()}</span> }
}

/// Варианты выпадающего списка пользователей; `selected` говорит, какой из них выбран.
fn user_options(
    vm: Vm,
    selected: impl Fn() -> Option<usize> + Copy + Send + Sync + 'static,
) -> impl IntoView {
    move || {
        vm.users()
            .into_iter()
            .map(|user| {
                let id = user.user_id;
                view! {
                    <option value=id.to_string() selected=move || selected() == Some(id)>
                        {format!("{} ({})", user.user_name, user.role)}
                    </option>
                }
            })
            .collect_view()
    }
}

fn optional_time(secs: Option<u64>) -> String {
    secs.map_or_else(|| "—".to_owned(), format_utc)
}

/// Отправка формы: браузеру перезагружать страницу не нужно, команду выполняет модель.
fn submit(event: SubmitEvent, command: impl Future<Output = ()> + 'static) {
    event.prevent_default();
    spawn_local(command);
}

fn agent_name(vm: Vm, agent: Option<usize>) -> String {
    agent
        .and_then(|id| vm.user_name(id))
        .unwrap_or_else(|| "—".to_owned())
}

/// Пауза поверх `setTimeout` браузера — таймер для повторного подключения.
async fn sleep(duration: Duration) {
    let (done, elapsed) = futures::channel::oneshot::channel();
    set_timeout(
        move || {
            let _ = done.send(());
        },
        duration,
    );
    let _ = elapsed.await;
}

/// Секунды Unix-времени как дата и время UTC: `2026-10-27 18:42 UTC`.
fn format_utc(secs: u64) -> String {
    // Перевод дней от эпохи в григорианскую дату (алгоритм Говарда Хиннанта).
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        time / 3_600,
        time % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::format_utc;

    #[test]
    fn unix_seconds_are_shown_as_utc_date_and_time() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(format_utc(1_793_134_069), "2026-10-27 20:47 UTC");
    }
}
