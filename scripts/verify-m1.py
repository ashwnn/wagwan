#!/usr/bin/env python3
"""Run M1's local synthetic-feed ingestion verification.

The default run is the full one-hour M1 gate. Use --smoke for a shorter
plumbing check; smoke runs can never report the one-hour gate as passed.
"""
import argparse
from datetime import datetime, timezone
import http.server
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request
import secrets


class FeedServer(http.server.BaseHTTPRequestHandler):
    modified = 0
    unchanged = 0

    def do_GET(self):
        if self.path == '/broken':
            self.send_error(503)
            return
        if self.path.startswith('/feed/'):
            if self.headers.get('If-None-Match') == '"fixture-v1"':
                type(self).unchanged += 1
                self.send_response(304)
                self.end_headers()
                return
            type(self).modified += 1
            item = self.path.rsplit('/', 1)[1]
            text = ('The city council approved a new housing proposal on Tuesday. '
                    'The proposal includes public parks and transport improvements. ' * 6)
            body = (f'<rss version="2.0"><channel><title>Fixture {item}</title>'
                    f'<link>{self.base}</link><description>Manual check</description>'
                    f'<item><guid>fixture-{item}</guid><title>Proposal {item}</title>'
                    f'<link>{self.base}/blocked/{item}</link>'
                    f'<description>{text}</description></item></channel></rss>').encode()
            self.send_response(200)
            self.send_header('ETag', '"fixture-v1"')
            self.send_header('Content-Type', 'application/rss+xml')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_error(403)  # Exercise feed-text fallback for article pages.

    def log_message(self, *_):
        pass


def article_count(db_path):
    if not db_path.exists():
        return 0
    with sqlite3.connect(db_path, timeout=2) as db:
        try:
            return db.execute(
                'SELECT count(*) FROM articles WHERE length(text)>300'
            ).fetchone()[0]
        except sqlite3.OperationalError:
            return 0


def make_output_dir(base):
    run_name = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    run_name += f'-{os.getpid()}-{secrets.token_hex(3)}'
    output = base / run_name
    output.mkdir(parents=True, exist_ok=False)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/wagwan')
    parser.add_argument(
        '--smoke', action='store_true',
        help='run a short plumbing check instead of the one-hour M1 gate',
    )
    parser.add_argument(
        '--smoke-seconds', type=int, default=120,
        help='duration for --smoke (default: 120 seconds)',
    )
    parser.add_argument(
        '--output-dir', type=Path, default=Path('.build/m1-verification'),
        help='base directory for retained per-run artifacts',
    )
    args = parser.parse_args()
    if args.smoke_seconds < 1:
        parser.error('--smoke-seconds must be positive')

    binary = Path(args.binary).resolve()
    if not binary.is_file():
        parser.error(f'binary does not exist: {binary}')
    output_dir = make_output_dir(args.output_dir.resolve())
    db_dir = output_dir / 'data'
    db_dir.mkdir()
    db_path = db_dir / 'wagwan.sqlite3'
    logs_path = output_dir / 'server.log'
    opml_path = output_dir / 'feeds.opml'
    summary_path = output_dir / 'summary.json'
    expected_runtime = args.smoke_seconds if args.smoke else 3600
    started_at = datetime.now(timezone.utc).isoformat()
    started = time.monotonic()
    process = None
    fixture = None
    fixture_thread = None
    status = 'failure'
    failure = None
    articles = 0
    run_exception = None
    FeedServer.modified = 0
    FeedServer.unchanged = 0

    def phase(message):
        print(f'[{time.monotonic() - started:6.1f}s] {message}', flush=True)

    def cleanup():
        nonlocal process, fixture, fixture_thread
        if process is not None and process.poll() is None:
            phase('Stopping Wagwan process')
            try:
                process.send_signal(signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                phase('Wagwan did not stop in 10 seconds; killing it')
                process.kill()
                process.wait()
        if fixture is not None:
            phase('Stopping local fixture server')
            if fixture_thread is not None and fixture_thread.is_alive():
                fixture.shutdown()
            fixture.server_close()
            if fixture_thread is not None:
                fixture_thread.join(timeout=5)
            fixture = None
            fixture_thread = None

    try:
        phase(f"Starting {'short smoke check' if args.smoke else 'full one-hour M1 gate'} ({expected_runtime}s)")
        phase(f'Creating retained artifacts in {output_dir}')
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        phase('Starting local 50-feed fixture server')
        fixture = http.server.ThreadingHTTPServer(('127.0.0.1', 0), FeedServer)
        FeedServer.base = f'http://127.0.0.1:{fixture.server_port}'
        fixture_thread = threading.Thread(target=fixture.serve_forever, daemon=True)
        fixture_thread.start()

        # Reserve a second port for Wagwan, then hand it to the child process.
        reservation = http.server.HTTPServer(('127.0.0.1', 0), FeedServer)
        app_port = reservation.server_port
        reservation.server_close()
        base = f'http://127.0.0.1:{app_port}'
        env = dict(os.environ, WAGWAN_DATA_DIR=str(db_dir),
                   WAGWAN_BIND=f'127.0.0.1:{app_port}',
                   WAGWAN_FETCH_MIN_INTERVAL='30s', WAGWAN_FETCH_MAX_INTERVAL='60s')
        opml_path.write_text('<opml version="2.0"><body>' + ''.join(
            f'<outline text="Feed {i}" xmlUrl="{FeedServer.base}/feed/{i}"/>'
            for i in range(50)) + '</body></opml>', encoding='utf-8')

        phase('Importing 50 fixture feeds')
        subprocess.run([str(binary), 'import-opml', str(opml_path)], env=env, check=True)

        phase(f'Starting Wagwan; server log: {logs_path}')
        logs = open(logs_path, 'w+', encoding='utf-8')
        process = subprocess.Popen([str(binary), 'serve'], env=env,
                                   stdout=logs, stderr=subprocess.STDOUT)
        try:
            phase('Waiting for /healthz')
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError('Wagwan exited during startup')
                try:
                    response = client.open(base + '/healthz', timeout=1)
                    if response.read() == b'ok':
                        break
                except (OSError, urllib.error.URLError):
                    time.sleep(.1)
            else:
                raise RuntimeError('Health endpoint did not become ready')

            phase('Adding a deliberately failing feed')
            data = urllib.parse.urlencode({'url': FeedServer.base + '/broken'}).encode()
            client.open(base + '/settings/feeds', data=data, timeout=5).close()
            phase(f'Ingestion observation started; progress reports every 30 seconds')
            observation_started = time.monotonic()
            next_report = 0
            while True:
                elapsed = time.monotonic() - observation_started
                if process.poll() is not None:
                    raise RuntimeError('Wagwan exited during ingestion')
                if elapsed >= expected_runtime:
                    break
                if elapsed >= next_report:
                    articles = article_count(db_path)
                    phase(f'Progress: {elapsed:.0f}s/{expected_runtime}s elapsed; '
                          f'{articles}/50 articles with text; '
                          f'{FeedServer.unchanged} conditional 304 requests')
                    next_report = elapsed + 30
                time.sleep(min(1.0, max(0.0, expected_runtime - elapsed)))

            articles = article_count(db_path)
            page = client.open(base + '/settings', timeout=5).read().decode()
            assert articles == 50, f'Expected 50 articles with text, got {articles}'
            assert 'Failing:' in page and 'Retry' in page
            if args.smoke:
                status = 'smoke_pass'
                phase(f'SMOKE PASS: plumbing checks passed after {elapsed:.0f}s; '
                      'the one-hour M1 gate was not run')
            else:
                assert FeedServer.unchanged > 0, 'No conditional 304 observed during run'
                logs.flush()
                logs.seek(0)
                log_text = logs.read()
                assert 'Feed not modified (304)' in log_text
                status = 'success'
                phase(f'GATE PASS: full one-hour verification passed after {elapsed:.0f}s')
        finally:
            logs.close()
    except KeyboardInterrupt:
        status = 'canceled'
        failure = 'Interrupted by SIGINT'
        phase('CANCELED: verification interrupted; no success is claimed')
    except Exception as exc:  # Keep failure details in the retained summary.
        failure = f'{type(exc).__name__}: {exc}'
        phase(f'FAILURE: {failure}')
        run_exception = exc
    finally:
        cleanup()
        elapsed = time.monotonic() - started
        summary = {
            'status': status,
            'gate': 'full_one_hour' if not args.smoke else 'short_smoke',
            'one_hour_gate_passed': status == 'success',
            'expected_runtime_seconds': expected_runtime,
            'elapsed_seconds': round(elapsed, 2),
            'articles_with_text': article_count(db_path),
            'conditional_304_requests': FeedServer.unchanged,
            'feed_200_requests': FeedServer.modified,
            'failure': failure,
            'started_at': started_at,
            'finished_at': datetime.now(timezone.utc).isoformat(),
            'artifacts': {
                'database': str(db_path),
                'server_log': str(logs_path),
                'opml_fixture': str(opml_path),
                'summary': str(summary_path),
            },
        }
        summary_path.write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
        print(f"{status.upper()}: artifacts retained in {output_dir}", flush=True)
        print(f"Database: {db_path}\nServer log: {logs_path}\nSummary: {summary_path}", flush=True)
    if run_exception is not None:
        raise run_exception
    return 130 if status == 'canceled' else (0 if status in ('success', 'smoke_pass') else 1)


if __name__ == '__main__':
    sys.exit(main())
