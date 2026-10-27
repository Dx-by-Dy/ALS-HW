//! Локальная копия read-модели сервера. Её поддерживает long polling: каждый ответ
//! `GET /api/changes` приносит изменённые записи, и копия их применяет.

use std::collections::BTreeMap;

use api::{ChangesDto, Cursor, TenantDto, TicketDto, UserDto};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replica {
    /// Позиция в потоке изменений сервера; `None`, пока ничего не получено.
    cursor: Option<Cursor>,
    tickets: BTreeMap<usize, TicketDto>,
    tenants: BTreeMap<usize, TenantDto>,
    users: BTreeMap<usize, UserDto>,
}

impl Replica {
    /// Применяет ответ long polling. Полный ответ заменяет всё: клиент новый
    /// или сервер перезапускался. Иначе записи заменяются по одной.
    pub fn apply(&mut self, changes: ChangesDto) {
        self.cursor = Some(changes.cursor());
        if changes.full {
            self.tickets.clear();
            self.tenants.clear();
            self.users.clear();
        }
        for ticket in changes.tickets {
            self.tickets.insert(ticket.ticket_id, ticket);
        }
        for tenant in changes.tenants {
            self.tenants.insert(tenant.tenant_id, tenant);
        }
        for user in changes.users {
            self.users.insert(user.user_id, user);
        }
    }

    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    pub fn version(&self) -> Option<u64> {
        self.cursor.map(|c| c.version)
    }

    /// Видны ли в копии результаты команды с версией `version`.
    pub fn has_reached(&self, version: u64) -> bool {
        self.version().is_some_and(|v| v >= version)
    }

    /// Тикеты по возрастанию номера.
    pub fn tickets(&self) -> impl Iterator<Item = &TicketDto> {
        self.tickets.values()
    }

    pub fn ticket(&self, ticket_id: usize) -> Option<&TicketDto> {
        self.tickets.get(&ticket_id)
    }

    pub fn tenants(&self) -> impl Iterator<Item = &TenantDto> {
        self.tenants.values()
    }

    pub fn tenant(&self, tenant_id: usize) -> Option<&TenantDto> {
        self.tenants.get(&tenant_id)
    }

    pub fn users(&self) -> impl Iterator<Item = &UserDto> {
        self.users.values()
    }

    pub fn user(&self, user_id: usize) -> Option<&UserDto> {
        self.users.get(&user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::{changes, ticket, user};
    use api::{Role, TicketStatus};

    fn ids(replica: &Replica) -> Vec<usize> {
        replica.tickets().map(|t| t.ticket_id).collect()
    }

    #[test]
    fn empty_replica_has_no_position() {
        let replica = Replica::default();
        assert_eq!(replica.cursor(), None);
        assert!(!replica.has_reached(0));
    }

    #[test]
    fn delta_replaces_changed_records_and_keeps_others() {
        let mut replica = Replica::default();
        replica.apply(ChangesDto {
            users: vec![user(1, "Alice", Role::Agent)],
            ..changes(
                1,
                10,
                true,
                vec![ticket(1, TicketStatus::Open), ticket(2, TicketStatus::Open)],
            )
        });
        replica.apply(changes(
            1,
            12,
            false,
            vec![
                ticket(2, TicketStatus::Closed),
                ticket(3, TicketStatus::Open),
            ],
        ));

        assert_eq!(ids(&replica), [1, 2, 3]);
        assert_eq!(replica.ticket(2).unwrap().status, TicketStatus::Closed);
        assert_eq!(replica.user(1).unwrap().user_name, "Alice");
        assert_eq!(
            replica.cursor(),
            Some(Cursor {
                epoch: 1,
                version: 12
            })
        );
        assert!(replica.has_reached(12));
        assert!(!replica.has_reached(13));
    }

    #[test]
    fn full_answer_replaces_everything() {
        let mut replica = Replica::default();
        replica.apply(changes(
            1,
            30,
            true,
            vec![ticket(1, TicketStatus::Open), ticket(2, TicketStatus::Open)],
        ));
        // Сервер перезапустился: новая эпоха, версия меньше, тикета 2 больше нет.
        replica.apply(changes(2, 5, true, vec![ticket(1, TicketStatus::Closed)]));

        assert_eq!(ids(&replica), [1]);
        assert_eq!(replica.version(), Some(5));
    }
}
