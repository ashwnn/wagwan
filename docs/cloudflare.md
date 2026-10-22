# Remote access with Cloudflare Tunnel and Access

Wagwan has no built-in login. Put Cloudflare Access in front of it before making it reachable from outside your home network.

1. In Cloudflare Zero Trust, create a Tunnel, choose Docker, and copy its token.
2. Add a public hostname such as `news.example.com` and route it to `http://wagwan:8080`.
3. Create a self-hosted Access application for that hostname. Add a policy that allows only your email address, using a one-time PIN or your identity provider.
4. Start the tunnel profile with `CLOUDFLARE_TUNNEL_TOKEN=... docker compose --profile tunnel up -d`.
5. Open the hostname on your phone, sign in through Access, then optionally add the page to your home screen.

The compose file binds port 8080 to localhost. Keep that binding local and never publish port 8080 directly to the internet. Cloudflare Access is the authentication boundary; Wagwan does not validate the Access JWT itself.
