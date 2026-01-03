# HTTP API Implementation Plan for Petit Scheduler

## Overview
Add an HTTP API server that runs alongside the scheduler, enabling external CLI/tooling to trigger jobs and query status.

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/jobs/{job_id}/trigger` | Trigger a manual job run, returns `RunId` |
| `GET` | `/api/jobs` | List all registered jobs |
| `GET` | `/api/jobs/{job_id}` | Get job details |
| `GET` | `/api/jobs/{job_id}/runs` | List recent runs for a job (with `?limit=N`) |
| `GET` | `/api/runs/{run_id}` | Get run status and details |
| `GET` | `/api/runs/{run_id}/tasks` | Get task states for a run |
| `GET` | `/api/health` | Health check endpoint |
| `GET` | `/api/scheduler/state` | Get scheduler state (running/paused) |
| `POST` | `/api/scheduler/pause` | Pause scheduled triggers |
| `POST` | `/api/scheduler/resume` | Resume scheduled triggers |

## Implementation Steps

### 1. Add dependencies to Cargo.toml
- `axum` - HTTP framework (integrates well with existing tokio runtime)
- `tower-http` - Middleware (CORS, tracing)

### 2. Create API module structure
```
src/api/
├── mod.rs          # Module exports, router construction
├── handlers.rs     # Request handlers
├── responses.rs    # JSON response types
└── errors.rs       # Error type -> HTTP status mapping
```

### 3. Refactor storage to be shareable
Currently `Scheduler::new()` takes ownership of storage. Need to:
- Change to `Arc<S>` where `S: Storage`
- Pass same `Arc<S>` to both scheduler and API layer

**Files to modify:**
- `src/scheduler/engine.rs` - Change `storage: S` to `storage: Arc<S>`
- `src/main.rs` - Wrap storage in Arc before passing

### 4. Create shared application state
```rust
pub struct ApiState<S: Storage> {
    pub handle: SchedulerHandle,
    pub storage: Arc<S>,
    pub jobs: Arc<HashMap<JobId, Job>>,  // For job metadata lookup
}
```

### 5. Implement handlers
Map existing storage/handle methods to HTTP endpoints with proper error handling.

### 6. Add CLI flags for API server
- API server enabled by default
- `--no-api` flag to disable HTTP server
- `--api-port` to configure port (default: 8565)
- `--api-host` to configure bind address (default: 127.0.0.1)

### 7. Update main.rs to spawn API server
Run HTTP server as separate tokio task alongside scheduler.

## Error Mapping

| Error | HTTP Status |
|-------|-------------|
| `JobNotFound` | 404 |
| `StorageError::NotFound` | 404 |
| `DependencyNotSatisfied` | 409 Conflict |
| `MaxConcurrentRunsExceeded` | 429 Too Many Requests |
| `NotRunning` | 503 Service Unavailable |
| Other | 500 Internal Server Error |

## Files to Create/Modify

| File | Action |
|------|--------|
| `src/api/mod.rs` | Create |
| `src/api/handlers.rs` | Create |
| `src/api/responses.rs` | Create |
| `src/api/errors.rs` | Create |
| `src/lib.rs` | Add `pub mod api;` |
| `src/main.rs` | Add CLI flags, spawn API server |
| `src/scheduler/engine.rs` | Refactor to use `Arc<S>` |
| `Cargo.toml` | Add axum, tower-http |

## Security Considerations

**IMPORTANT**: The Petit API does not include built-in authentication or authorization. It is designed to run as a local service for trusted environments.

### Production Deployment

For production use, you should secure the API using one or more of the following approaches:

#### 1. Network Isolation (Recommended)

Bind the API to localhost only and use SSH tunneling or a VPN for remote access:

```bash
# Bind to localhost (default)
petit run jobs/ --api-host 127.0.0.1 --api-port 8565

# Access remotely via SSH tunnel
ssh -L 8565:localhost:8565 user@remote-host
```

This is the **most secure** approach as the API is never exposed to the network.

#### 2. Reverse Proxy with Authentication

Use a reverse proxy (nginx, Caddy, Traefik) with authentication:

**nginx example with basic auth:**
```nginx
server {
    listen 443 ssl;
    server_name scheduler.example.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    auth_basic "Scheduler API";
    auth_basic_user_file /etc/nginx/.htpasswd;

    location /api/ {
        proxy_pass http://127.0.0.1:8565;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

**Caddy example with authentication:**
```
scheduler.example.com {
    basicauth /* {
        user $2a$14$hashed_password
    }
    reverse_proxy localhost:8565
}
```

#### 3. Firewall Rules

Restrict access to the API port using firewall rules:

```bash
# Allow only specific IP addresses
sudo ufw allow from 192.168.1.0/24 to any port 8565

# Or use iptables
sudo iptables -A INPUT -p tcp -s 192.168.1.0/24 --dport 8565 -j ACCEPT
sudo iptables -A INPUT -p tcp --dport 8565 -j DROP
```

#### 4. Container Network Policies

When running in containers (Docker, Kubernetes), use network policies to restrict access:

**Docker Compose example:**
```yaml
services:
  petit:
    image: petit:latest
    networks:
      - internal
    # Don't expose ports externally
    expose:
      - "8565"

  nginx:
    image: nginx:latest
    ports:
      - "443:443"
    networks:
      - internal
      - external
    # nginx handles external access with auth

networks:
  internal:
    internal: true
  external:
```

### Security Checklist

Before deploying to production:

- [ ] API is bound to localhost or a private network interface
- [ ] If exposed, authentication is enforced via reverse proxy
- [ ] TLS/SSL is enabled for encrypted communication
- [ ] Firewall rules restrict access to authorized IPs only
- [ ] Access logs are monitored for suspicious activity
- [ ] API port is not exposed to the public internet
- [ ] Network segmentation isolates the scheduler from untrusted networks

### Future Enhancements

The following authentication features may be considered for future development:

- API key authentication
- JWT token support
- Role-based access control (RBAC)
- Audit logging for all API operations
- Rate limiting per client

For now, use the approaches above to secure your deployment.

## Example Usage

```bash
# Start scheduler (API enabled by default on port 8565)
petit --db petit.db run examples/jobs

# Start without API
petit --db petit.db run examples/jobs --no-api

# Custom port
petit --db petit.db run examples/jobs --api-port 9000

# Trigger job from another terminal (local only)
curl -X POST http://localhost:8565/api/jobs/hello_world/trigger

# Check run status
curl http://localhost:8565/api/runs/<run-id>

# List jobs
curl http://localhost:8565/api/jobs
```
