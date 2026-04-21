use crate::api::notifications::Notification;

pub fn row_to_notification(row: crate::storage::NotificationRow) -> Notification {
    Notification {
        id: row.id,
        kind: row.kind,
        title: row.title,
        body: row.body,
        target_type: row.target_type,
        target_id: row.target_id,
        href: row.href,
        payload: row.payload,
        occurrence_count: row.occurrence_count,
        viewed_at: row.viewed_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
