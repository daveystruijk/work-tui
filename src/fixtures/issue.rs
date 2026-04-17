use std::collections::BTreeMap;

use serde_json::json;

pub(crate) fn test_issue() -> crate::apis::jira::Issue {
    crate::apis::jira::Issue {
        self_link: "http://localhost/rest/api/2/issue/TEST-123".to_string(),
        key: "TEST-123".to_string(),
        id: "10001".to_string(),
        fields: BTreeMap::from([
            ("summary".to_string(), json!("Snapshot the sidebar render")),
            (
                "description".to_string(),
                json!("Confirm sidebar, list, and command bar layouts."),
            ),
            (
                "status".to_string(),
                json!({
                    "description": "",
                    "iconUrl": "",
                    "id": "3",
                    "name": "In Progress",
                    "self": "http://localhost/status/3"
                }),
            ),
            (
                "issuetype".to_string(),
                json!({
                    "description": "",
                    "iconUrl": "",
                    "id": "10004",
                    "name": "Task",
                    "self": "http://localhost/issuetype/10004",
                    "subtask": false
                }),
            ),
            (
                "assignee".to_string(),
                json!({
                    "accountId": "acc-1",
                    "active": true,
                    "avatarUrls": null,
                    "displayName": "Casey Dev",
                    "emailAddress": "casey@example.com",
                    "key": null,
                    "name": null,
                    "self": "http://localhost/user/1",
                    "timeZone": null
                }),
            ),
            (
                "creator".to_string(),
                json!({
                    "accountId": "acc-2",
                    "active": true,
                    "avatarUrls": null,
                    "displayName": "Riley Author",
                    "emailAddress": "riley@example.com",
                    "key": null,
                    "name": null,
                    "self": "http://localhost/user/2",
                    "timeZone": null
                }),
            ),
            (
                "reporter".to_string(),
                json!({
                    "accountId": "acc-3",
                    "active": true,
                    "avatarUrls": null,
                    "displayName": "Jordan QA",
                    "emailAddress": "jordan@example.com",
                    "key": null,
                    "name": null,
                    "self": "http://localhost/user/3",
                    "timeZone": null
                }),
            ),
        ]),
    }
}
