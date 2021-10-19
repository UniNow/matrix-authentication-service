// Copyright 2021 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::convert::TryFrom;

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use mas_data_model::{AccessToken, Authentication, BrowserSession, Client, Session, User};
use sqlx::PgExecutor;
use thiserror::Error;

use crate::storage::{DatabaseInconsistencyError, IdAndCreationTime, PostgresqlBackend};

pub async fn add_access_token(
    executor: impl PgExecutor<'_>,
    oauth2_session_id: i64,
    token: &str,
    expires_after: Duration,
) -> anyhow::Result<AccessToken<PostgresqlBackend>> {
    // Checked convertion of duration to i32, maxing at i32::MAX
    let expires_after_seconds = i32::try_from(expires_after.num_seconds()).unwrap_or(i32::MAX);

    let res = sqlx::query_as!(
        IdAndCreationTime,
        r#"
            INSERT INTO oauth2_access_tokens
                (oauth2_session_id, token, expires_after)
            VALUES
                ($1, $2, $3)
            RETURNING
                id, created_at
        "#,
        oauth2_session_id,
        token,
        expires_after_seconds,
    )
    .fetch_one(executor)
    .await
    .context("could not insert oauth2 access token")?;

    Ok(AccessToken {
        data: res.id,
        expires_after,
        token: token.to_string(),
        jti: format!("{}", res.id),
        created_at: res.created_at,
    })
}

#[derive(Debug)]
pub struct OAuth2AccessTokenLookup {
    access_token_id: i64,
    access_token: String,
    access_token_expires_after: i32,
    access_token_created_at: DateTime<Utc>,
    session_id: i64,
    client_id: String,
    scope: String,
    redirect_uri: String,
    nonce: Option<String>,
    user_session_id: i64,
    user_session_created_at: DateTime<Utc>,
    user_id: i64,
    user_username: String,
    user_session_last_authentication_id: Option<i64>,
    user_session_last_authentication_created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
#[error("failed to lookup access token")]
pub enum AccessTokenLookupError {
    Database(#[from] sqlx::Error),
    Inconsistency(#[from] DatabaseInconsistencyError),
}

impl AccessTokenLookupError {
    #[must_use]
    pub fn not_found(&self) -> bool {
        matches!(
            self,
            &AccessTokenLookupError::Database(sqlx::Error::RowNotFound)
        )
    }
}

pub async fn lookup_active_access_token(
    executor: impl PgExecutor<'_>,
    token: &str,
) -> Result<(AccessToken<PostgresqlBackend>, Session<PostgresqlBackend>), AccessTokenLookupError> {
    let res = sqlx::query_as!(
        OAuth2AccessTokenLookup,
        r#"
            SELECT
                at.id              AS "access_token_id",
                at.token           AS "access_token",
                at.expires_after   AS "access_token_expires_after",
                at.created_at      AS "access_token_created_at",
                os.id              AS "session_id!",
                os.client_id       AS "client_id!",
                os.scope           AS "scope!",
                os.redirect_uri    AS "redirect_uri!",
                os.nonce           AS "nonce",
                us.id              AS "user_session_id!",
                us.created_at      AS "user_session_created_at!",
                 u.id              AS "user_id!",
                 u.username        AS "user_username!",
                usa.id             AS "user_session_last_authentication_id?",
                usa.created_at     AS "user_session_last_authentication_created_at?"

            FROM oauth2_access_tokens at
            INNER JOIN oauth2_sessions os
              ON os.id = at.oauth2_session_id
            INNER JOIN user_sessions us
              ON us.id = os.user_session_id
            INNER JOIN users u
              ON u.id = us.user_id
            LEFT JOIN user_session_authentications usa
              ON usa.session_id = us.id

            WHERE at.token = $1
              AND at.created_at + (at.expires_after * INTERVAL '1 second') >= now()
              AND us.active

            ORDER BY usa.created_at DESC
            LIMIT 1
        "#,
        token,
    )
    .fetch_one(executor)
    .await?;

    let access_token = AccessToken {
        data: res.access_token_id,
        jti: format!("{}", res.access_token_id),
        token: res.access_token,
        created_at: res.access_token_created_at,
        expires_after: Duration::seconds(res.access_token_expires_after.into()),
    };

    let client = Client {
        data: (),
        client_id: res.client_id,
    };

    let user = User {
        data: res.user_id,
        username: res.user_username,
        sub: format!("fake-sub-{}", res.user_id),
    };

    let last_authentication = match (
        res.user_session_last_authentication_id,
        res.user_session_last_authentication_created_at,
    ) {
        (None, None) => None,
        (Some(id), Some(created_at)) => Some(Authentication {
            data: id,
            created_at,
        }),
        _ => return Err(DatabaseInconsistencyError.into()),
    };

    let browser_session = Some(BrowserSession {
        data: res.user_session_id,
        created_at: res.user_session_created_at,
        user,
        last_authentication,
    });

    let scope = res.scope.parse().map_err(|_e| DatabaseInconsistencyError)?;

    let redirect_uri = res
        .redirect_uri
        .parse()
        .map_err(|_e| DatabaseInconsistencyError)?;

    let session = Session {
        data: res.session_id,
        client,
        browser_session,
        scope,
        redirect_uri,
        nonce: res.nonce,
    };

    Ok((access_token, session))
}

pub async fn revoke_access_token(executor: impl PgExecutor<'_>, id: i64) -> anyhow::Result<()> {
    let res = sqlx::query!(
        r#"
            DELETE FROM oauth2_access_tokens
            WHERE id = $1
        "#,
        id,
    )
    .execute(executor)
    .await
    .context("could not revoke access tokens")?;

    if res.rows_affected() == 1 {
        Ok(())
    } else {
        Err(anyhow::anyhow!("no row were affected when revoking token"))
    }
}

pub async fn cleanup_expired(executor: impl PgExecutor<'_>) -> anyhow::Result<u64> {
    let res = sqlx::query!(
        r#"
            DELETE FROM oauth2_access_tokens
            WHERE created_at + (expires_after * INTERVAL '1 second') + INTERVAL '15 minutes' < now()
        "#,
    )
    .execute(executor)
    .await
    .context("could not cleanup expired access tokens")?;

    Ok(res.rows_affected())
}
