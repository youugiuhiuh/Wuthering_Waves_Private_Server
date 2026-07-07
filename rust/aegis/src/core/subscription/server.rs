use std::os::unix::fs::PermissionsExt;
use tonic::{Request, Response, Status, transport::Server};

use crate::core::subscription::token::TokenManager;

pub mod proto {
    tonic::include_proto!("subscription");
}

use proto::{
    CreateTokenRequest, GetConfigsRequest, GetConfigsResponse, GetTokenInfoRequest,
    ListTokensRequest, ListTokensResponse, RevokeTokenRequest, RevokeTokenResponse, TokenInfo,
    TokenResponse, UpdateTokenConfigsRequest,
    subscription_service_server::{SubscriptionService, SubscriptionServiceServer},
    token_service_server::{TokenService, TokenServiceServer},
};

#[derive(Clone)]
pub struct SubGrpcServer {
    pub token_mgr: TokenManager,
}

#[tonic::async_trait]
impl SubscriptionService for SubGrpcServer {
    async fn get_configs(
        &self,
        request: Request<GetConfigsRequest>,
    ) -> Result<Response<GetConfigsResponse>, Status> {
        let req = request.into_inner();
        let configs = self
            .token_mgr
            .get_configs_for_token(&req.token)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(GetConfigsResponse { configs }))
    }

    async fn get_token_info(
        &self,
        request: Request<GetTokenInfoRequest>,
    ) -> Result<Response<TokenInfo>, Status> {
        let req = request.into_inner();
        let (info, cfg_count) = self
            .token_mgr
            .get_token_info(&req.token)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(TokenInfo {
            info: Some(info),
            subscription_url: format!("/sub/{}", &req.token),
            configs_count: cfg_count as i64,
        }))
    }
}

#[tonic::async_trait]
impl TokenService for SubGrpcServer {
    async fn create_token(
        &self,
        request: Request<CreateTokenRequest>,
    ) -> Result<Response<TokenResponse>, Status> {
        let req = request.into_inner();
        let token = self
            .token_mgr
            .create_token(&req.label, &req.config_ids)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(TokenResponse { info: Some(token) }))
    }

    async fn list_tokens(
        &self,
        request: Request<ListTokensRequest>,
    ) -> Result<Response<ListTokensResponse>, Status> {
        let req = request.into_inner();
        let (tokens, total) = self
            .token_mgr
            .list_tokens(req.page, req.page_size)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListTokensResponse {
            tokens,
            total: total as i32,
        }))
    }

    async fn revoke_token(
        &self,
        request: Request<RevokeTokenRequest>,
    ) -> Result<Response<RevokeTokenResponse>, Status> {
        let req = request.into_inner();
        self.token_mgr
            .revoke_token(&req.token)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(RevokeTokenResponse { success: true }))
    }

    async fn update_token_configs(
        &self,
        request: Request<UpdateTokenConfigsRequest>,
    ) -> Result<Response<TokenResponse>, Status> {
        let req = request.into_inner();
        let token = self
            .token_mgr
            .update_token(&req.token, &req.config_ids, req.expires_at)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(TokenResponse { info: Some(token) }))
    }
}

pub async fn start_grpc_server(
    socket_path: &str,
    token_mgr: TokenManager,
) -> Result<(), Box<dyn std::error::Error>> {
    if std::path::Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }
    let grpc_server = SubGrpcServer { token_mgr };
    let uds = tokio::net::UnixListener::bind(socket_path)?;
    // Set socket permissions to 0600 for security
    if let Ok(addr) = uds.local_addr()
        && let Some(path) = addr.as_pathname()
    {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);
    Server::builder()
        .add_service(SubscriptionServiceServer::new(grpc_server.clone()))
        .add_service(TokenServiceServer::new(grpc_server))
        .serve_with_incoming(incoming)
        .await?;
    Ok(())
}
