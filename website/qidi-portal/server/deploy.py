import datetime
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tarfile


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def install(source, target, mode=0o644):
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(target.name + '.qidi-new')
    shutil.copyfile(source, temporary)
    temporary.chmod(mode)
    temporary.replace(target)


def main():
    if os.geteuid() != 0:
        raise RuntimeError('需要 sudo 执行部署')
    stage = Path(sys.argv[1]).resolve()
    if stage.parent != Path('/home/ubuntu') or not stage.name.startswith('qidi-deploy-'):
        raise RuntimeError('部署目录不符合约定')
    portal = Path('/var/www/qidi-portal')
    download = Path('/var/www/qidiwork')
    configs = [Path('/etc/nginx/sites-enabled/qidi-portal'), Path('/etc/nginx/sites-enabled/qidiwork')]
    env_file = Path('/etc/qidi-analytics.env')
    service = Path('/etc/systemd/system/qidi-analytics.service')
    snippet = Path('/etc/nginx/snippets/qidi-analytics.conf')
    code = Path('/opt/qidi-analytics/analytics.py')
    roots = [portal, download]
    for root in roots:
        if not (root / 'index.html').is_file():
            raise RuntimeError(f'缺少现有站点 {root}')
    if shutil.disk_usage('/').free < 1500 * 1024 * 1024:
        raise RuntimeError('可用空间不足 1.5GB，停止部署')
    run('nginx', '-t')
    originals = {p: p.read_bytes() if p.exists() else None for p in [*configs, env_file, service, snippet, code]}
    config_texts = [p.read_text() for p in configs]
    include = '    include /etc/nginx/snippets/qidi-analytics.conf;'
    for text, root in zip(config_texts, roots):
        if text.count(f'    root {root};') != 1:
            raise RuntimeError(f'nginx 根目录标记不唯一 {root}')
    news = portal / 'data/news.json'
    latest = download / 'releases/latest.json'
    retained = {str(p): digest(p) for p in [news, latest] if p.exists()}
    release_before = {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for p in (download / 'releases').rglob('*') if p.is_file()}
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    backup = Path('/home/ubuntu/portal-backups') / stamp
    backup.mkdir(parents=True, mode=0o700)
    backup.chmod(0o700)
    archive = backup / 'sites-and-config.tar.gz'
    with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
        for path in [*roots, *configs, env_file, service, snippet, code]:
            if path.exists():
                tar.add(path, arcname=str(path).lstrip('/'))
    archive.chmod(0o600)
    with tarfile.open(archive) as tar:
        names = tar.getnames()
        for required in ['var/www/qidi-portal/index.html', 'var/www/qidiwork/index.html', 'etc/nginx/sites-enabled/qidi-portal']:
            if required not in names:
                raise RuntimeError('备份内容不完整')
    (backup / 'sha256.txt').write_text(digest(archive) + '  sites-and-config.tar.gz\n')
    print(f'BACKUP_OK {backup} ({archive.stat().st_size} bytes)', flush=True)
    was_active = subprocess.run(['systemctl', 'is-active', '--quiet', 'qidi-analytics']).returncode == 0
    static_originals = {}
    try:
        install(stage / 'server/analytics.py', code)
        if not env_file.exists():
            password, secret = secrets.token_urlsafe(32), secrets.token_urlsafe(48)
            fd = os.open(env_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'w') as stream:
                stream.write(f'QIDI_ADMIN_PASSWORD={password}\nQIDI_ANALYTICS_SECRET={secret}\nQIDI_ANALYTICS_DB=/var/lib/qidi-analytics/analytics.sqlite3\nQIDI_ALLOWED_ORIGINS=https://www.qidiai.ltd,https://qidiwork.qidiai.ltd\n')
        install(stage / 'server/qidi-analytics.service', service)
        install(stage / 'server/qidi-analytics.nginx', snippet)
        for path, text, root in zip(configs, config_texts, roots):
            if include not in text:
                text = text.replace(f'    root {root};', f'    root {root};\n{include}')
            path.write_text(text)
        run('nginx', '-t')
        run('systemctl', 'daemon-reload')
        run('systemctl', 'restart', 'qidi-analytics')
        run('systemctl', 'is-active', 'qidi-analytics')
        import time
        import urllib.error
        import urllib.request
        deadline = time.monotonic() + 10
        while True:
            try:
                urllib.request.urlopen('http://127.0.0.1:4180/api/analytics/report', timeout=2)
                raise RuntimeError('未授权报表意外成功')
            except urllib.error.HTTPError as error:
                if error.code != 401:
                    raise
                break
            except urllib.error.URLError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        for folder, destination in [('public', portal), ('download', download)]:
            for source in sorted((stage / folder).rglob('*')):
                if not source.is_file():
                    continue
                target = destination / source.relative_to(stage / folder)
                if target.is_symlink() or any(parent.is_symlink() for parent in target.parents if parent != destination):
                    raise RuntimeError(f'部署目标不允许符号链接 {target}')
                static_originals[target] = (target.read_bytes(), target.stat().st_mode & 0o777) if target.exists() else None
                install(source, target)
        run('systemctl', 'reload', 'nginx')
        run('systemctl', 'enable', 'qidi-analytics')
        release_after = {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for p in (download / 'releases').rglob('*') if p.is_file()}
        if release_after != release_before:
            raise RuntimeError('安装包目录发生意外变化')
        if latest.exists() and digest(latest) != retained[str(latest)]:
            raise RuntimeError('版本清单发生意外变化')
        print('DEPLOY_OK ' + json.dumps({'backup': str(backup), 'files': len(static_originals), 'releases_unchanged': True, 'news_unchanged': not news.exists() or digest(news) == retained.get(str(news)), 'service': 'active'}, ensure_ascii=False), flush=True)
    except Exception:
        if not was_active and service.exists():
            run('systemctl', 'stop', 'qidi-analytics')
            if originals[service] is None:
                run('systemctl', 'disable', 'qidi-analytics')
        for path, original in static_originals.items():
            if original is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(original[0])
                path.chmod(original[1])
        for path, data in originals.items():
            if data is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(data)
        run('systemctl', 'daemon-reload')
        if was_active:
            run('systemctl', 'restart', 'qidi-analytics')
        run('nginx', '-t')
        run('systemctl', 'reload', 'nginx')
        print(f'已恢复部署前文件；完整备份 {backup}', file=sys.stderr)
        raise


if __name__ == '__main__':
    main()
