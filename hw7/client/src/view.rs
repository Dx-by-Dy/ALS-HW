//! View: разметка формы. Своей логики здесь нет — только привязки к свойствам и командам
//! `TicketViewModel`.

use leptos::{prelude::*, task::spawn_local};

use crate::model::{Ticket, TicketStatus, UserId, http::BrowserHttpClient};
use crate::view_model::TicketViewModel;

type Vm = TicketViewModel<BrowserHttpClient>;

#[component]
pub fn TicketView(vm: Vm) -> impl IntoView {
    spawn_local(async move { vm.load().await });
    view! {
        <main class="app">
            <header class="topbar">
                <h1>"Support tickets"</h1>
                <span class="sync" class:visible=move || vm.is_busy()>
                    "Syncing with server…"
                </span>
            </header>
            <TenantBar vm />
            <div class="layout">
                <TicketList vm />
                <TicketDetails vm />
            </div>
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
        </section>
    }
}

fn ticket_row(vm: Vm, ticket: Ticket) -> impl IntoView {
    let id = ticket.ticket_id;
    let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
    view! {
        <tr class:selected=move || vm.is_selected(id) on:click=move |_| vm.select_ticket(id)>
            <td>{id.0}</td>
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
                            vm.select_user(event_target_value(&ev).parse().ok().map(UserId))
                        }
                    >
                        <option value="">"Choose a user"</option>
                        {move || {
                            vm.users()
                                .into_iter()
                                .map(|user| {
                                    let id = user.user_id;
                                    view! {
                                        <option
                                            value=id.0.to_string()
                                            selected=move || vm.selected_user_id() == Some(id)
                                        >
                                            {format!("{} ({})", user.user_name, user.role)}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
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

fn ticket_card(vm: Vm, ticket: Ticket) -> impl IntoView {
    let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
    view! {
        <div class="card">
            <div class="card-title">
                <h2>{format!("{} · {}", ticket.ticket_id, ticket.category)}</h2>
                {status_badge(ticket.status)}
            </div>
            <p class="description">{ticket.description}</p>
            <dl>
                <dt>"Tenant"</dt>
                <dd>{tenant}</dd>
                <dt>"Agent"</dt>
                <dd>{agent_name(vm, ticket.assigned_agent)}</dd>
            </dl>
        </div>
    }
}

fn status_badge(status: TicketStatus) -> impl IntoView {
    let class = match status {
        TicketStatus::Open => "badge open",
        TicketStatus::InProgress => "badge in-progress",
        TicketStatus::Closed => "badge closed",
    };
    view! { <span class=class>{status.to_string()}</span> }
}

fn agent_name(vm: Vm, agent: Option<UserId>) -> String {
    agent
        .and_then(|id| vm.user_name(id))
        .unwrap_or_else(|| "—".to_owned())
}
