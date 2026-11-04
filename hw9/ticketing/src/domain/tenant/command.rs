use super::TenantId;

/// Команды агрегата `Tenant`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantCommand {
    Register {
        tenant_id: TenantId,
        tenant_name: String,
    },
    /// Пересчитать скидку. Число открытых тикетов принадлежит другому агрегату,
    /// поэтому его передаёт сервис приложения.
    ApplyDiscount { open_tickets: usize },
}
