//! MCP Server Implementation
//!
//! The main server that handles MCP protocol communication.

use anyhow::{Context, Result};
use std::sync::Arc;

use super::protocol::ProtocolHandler;
use super::transport::StdioTransport;
use super::types::{JsonRpcRequest, RequestId};
use crate::tools::ToolRegistry;

/// MCP Server
pub struct McpServer {
    transport: StdioTransport,
    protocol: ProtocolHandler,
    registry: Arc<ToolRegistry>,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            transport: StdioTransport::new(),
            protocol: ProtocolHandler::new(),
            registry: Arc::new(registry),
        }
    }

    /// Run the server (main event loop)
    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("MCP server starting...");

        loop {
            // Read request
            let request = match self.transport.read_request().await {
                Ok(req) => req,
                Err(e) => {
                    if e.to_string().contains("EOF") {
                        tracing::info!("Client disconnected");
                        break;
                    }
                    tracing::error!(error = %e, "Failed to read request");
                    continue;
                }
            };

            // Handle request
            let response = self.handle_request(request).await;

            // Write response (notifications return None)
            if let Some(response) = response {
                if let Err(e) = self.transport.write_response(response).await {
                    tracing::error!(error = %e, "Failed to write response");
                }
            }
        }

        tracing::info!("MCP server stopped");
        Ok(())
    }

    /// Handle a JSON-RPC request
    async fn handle_request(&self, request: JsonRpcRequest) -> Option<super::types::JsonRpcResponse> {
        // Notifications have no id — process but don't respond
        let is_notification = request.id.is_none();

        // Validate request
        if let Err(e) = self.protocol.validate_request(&request) {
            return Some(self.protocol.create_error_response(
                request.id.unwrap_or(super::types::RequestId::Null), e,
            ));
        }

        // Route to appropriate handler
        let result = match request.method.as_str() {
            "initialize" => self.protocol.handle_initialize(
                request.id.unwrap_or(super::types::RequestId::Null),
            ),
            "ping" => self.protocol.handle_ping(
                request.id.unwrap_or(super::types::RequestId::Null),
            ),
            "tools/list" => {
                let tools = self.registry.list_tools();
                self.protocol.create_tool_list_response(
                    request.id.unwrap_or(super::types::RequestId::Null),
                    tools,
                )
            }
            "tools/call" => {
                match self
                    .handle_tool_call(
                        request.id.clone().unwrap_or(super::types::RequestId::Null),
                        request.params,
                    )
                    .await
                {
                    Ok(response) => response,
                    Err(e) => self.protocol.create_error_response(
                        request.id.unwrap_or(super::types::RequestId::Null),
                        e,
                    ),
                }
            }
            _ => {
                if is_notification {
                    tracing::debug!(method = %request.method, "Ignoring unknown notification");
                    return None;
                }
                let error =
                    super::types::JsonRpcError::method_not_found(&request.method);
                super::types::JsonRpcResponse::error(
                    request.id.unwrap_or(super::types::RequestId::Null),
                    error,
                )
            }
        };

        if is_notification {
            tracing::debug!(method = %request.method, "Processed notification");
            None
        } else {
            Some(result)
        }
    }

    /// Handle a tool call
    async fn handle_tool_call(
        &self,
        id: RequestId,
        params: Option<serde_json::Value>,
    ) -> Result<super::types::JsonRpcResponse> {
        // Parse tool call parameters
        let tool_call = self.protocol.parse_tool_call(params)?;

        tracing::info!(tool_name = %tool_call.name, "Executing tool");

        // Get the tool
        let tool = self
            .registry
            .get(&tool_call.name)
            .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", tool_call.name))?;

        // Execute the tool
        let result = tool
            .execute(tool_call.arguments)
            .await
            .context("Tool execution failed")?;

        // Create response
        Ok(self.protocol.create_tool_result_response(id, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speckit::SpecKitCli;
    use crate::tools::create_registry;

    #[test]
    fn test_server_creation() {
        let cli = SpecKitCli::new();
        let registry = create_registry(cli);
        let server = McpServer::new(registry);

        // Just ensure server can be created
        assert!(std::mem::size_of_val(&server) > 0);
    }
}
