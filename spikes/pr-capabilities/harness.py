#!/usr/bin/env python3
"""Disposable Git/gh capability experiment, not a production adapter."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import tempfile


def run(args, cwd=None, data=None, timeout=120, env=None):
    with subprocess.Popen(args, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, start_new_session=True, env=env) as process:
        try:
            stdout, stderr = process.communicate(data, timeout=timeout)
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise
        if process.returncode:
            raise subprocess.CalledProcessError(process.returncode, args, stdout, stderr)
        return stdout


def git(repo, *args, data=None):
    return run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false',
                '-c', 'diff.external=', '-c', 'core.autocrlf=false',
                '-c', 'diff.algorithm=myers', '-c', 'diff.indentHeuristic=false',
                '-C', str(repo), *args], data=data, env={**os.environ,
                'GIT_AUTHOR_DATE': '2000-01-01T00:00:00Z',
                'GIT_COMMITTER_DATE': '2000-01-01T00:00:00Z'})


def digest(data):
    return hashlib.sha256(data).hexdigest()


def snapshot(repo):
    """No status/refresh: compare raw index and every worktree byte/link/mode."""
    files = {}
    for path in sorted(repo.rglob('*')):
        if '.git' in path.relative_to(repo).parts:
            continue
        if path.is_symlink():
            value = os.readlink(path).encode()
        elif path.is_file():
            value = path.read_bytes()
        else:
            continue
        files[str(path.relative_to(repo))] = [path.lstat().st_mode, digest(value)]
    index = Path(git(repo, 'rev-parse', '--absolute-git-dir').decode().strip()) / 'index'
    return {'head': git(repo, 'rev-parse', 'HEAD').decode().strip(),
            'branch': git(repo, 'symbolic-ref', 'HEAD').decode().strip(),
            'index': digest(index.read_bytes()) if index.exists() else None, 'files': files}


def fetch(repo, remote, *refs, timeout=120):
    # No destination refspec, configured remote, tags, FETCH_HEAD or submodule writes.
    return run(['git', '-c', 'core.hooksPath=/dev/null', '-C', str(repo),
                'fetch', '--no-tags', '--no-write-fetch-head', '--no-auto-maintenance',
                '--no-recurse-submodules', '--', str(remote), *refs], timeout=timeout)


def acquire(repo, remote, base, head, head_ref=None):
    for sha, ref in [(base, base), (head, head_ref or head)]:
        if not re.fullmatch(r'[0-9a-f]{40}', sha):
            raise ValueError('expected full SHA-1 object identity')
        try:
            git(repo, 'cat-file', '-e', sha + '^{commit}')
        except subprocess.CalledProcessError:
            fetch(repo, remote, ref)
        # A moved PR ref must not silently replace the pinned head.
        git(repo, 'cat-file', '-e', sha + '^{commit}')
    return git(repo, 'merge-base', base, head).decode().strip()


def comparison(repo, identity, base, head):
    merge_base = git(repo, 'merge-base', base, head).decode().strip()
    patch = git(repo, '-c', 'core.quotePath=true', 'diff', '--no-ext-diff', '--no-textconv',
                '--no-color', '--src-prefix=a/', '--dst-prefix=b/', '--unified=3',
                '--find-renames=50%', merge_base, head, '--')
    semantics = {'version': 'spike-1', 'identity': identity, 'base': base, 'head': head,
                 'merge_base': merge_base, 'diff': 'git:u3:rename50:noext:notextconv',
                 'patch_sha256': digest(patch)}
    context = digest(json.dumps(semantics, sort_keys=True, separators=(',', ':')).encode())
    # Parse one file at a time using NUL-separated raw metadata, never quoted path headers.
    raw = git(repo, 'diff', '--raw', '-z', '--no-abbrev', '--no-ext-diff', '--no-textconv',
              '--find-renames=50%', merge_base, head, '--').split(b'\0')
    files = {}
    i = 0
    while i < len(raw) and raw[i]:
        metadata = raw[i].decode().split()
        old_mode, new_mode = metadata[0][1:], metadata[1]
        status = metadata[4]
        old_path = raw[i + 1].decode()
        new_path = raw[i + 2].decode() if status.startswith(('R', 'C')) else old_path
        i += 3 if status.startswith(('R', 'C')) else 2
        file_patch = git(repo, 'diff', '--no-ext-diff', '--no-textconv', '--no-color',
                         '--unified=3', '--find-renames=50%', merge_base, head,
                         '--', ':(literal)' + old_path, ':(literal)' + new_path)
        unsupported = ('submodule' if '160000' in (old_mode, new_mode) else
                       'symlink' if '120000' in (old_mode, new_mode) else
                       'binary' if b'Binary files ' in file_patch else None)
        lines = []
        old = new = None
        for line in file_patch.split(b'\n'):
            match = re.match(rb'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@', line)
            if match:
                old, new = map(int, match.groups())
            elif old is not None and line[:1] in (b' ', b'-', b'+'):
                kind = {b' ': 'context', b'-': 'delete', b'+': 'add'}[line[:1]]
                lines.append({'old': old if kind != 'add' else None,
                              'new': new if kind != 'delete' else None, 'kind': kind})
                old += kind != 'add'
                new += kind != 'delete'
        files[new_path] = {'old_path': old_path, 'status': status,
                           'unsupported': unsupported, 'lines': [] if unsupported else lines}
    return {**semantics, 'context': context, 'files': files}


def eligible(file, side, start, end, suggestion=False):
    if file['unsupported'] or side not in ('old', 'new') or start < 1 or end < start:
        return False
    if suggestion and side != 'new':
        return False
    available = {line[side] for line in file['lines'] if line[side] is not None}
    return all(line in available for line in range(start, end + 1))



def source(repo, revision, path, start, end):
    if (not path or path.startswith('/') or '\\' in path
            or any(part in ('', '.', '..') for part in path.split('/'))
            or start < 1 or end < start):
        raise ValueError('invalid source locator')
    entry = git(repo, 'ls-tree', '-z', revision, '--', ':(literal)' + path)
    if not entry or entry.split(b' ', 1)[0] not in (b'100644', b'100755'):
        raise ValueError('not a regular source blob')
    blob = git(repo, 'show', revision + ':' + path)
    if b'\0' in blob or blob.startswith(b'version https://git-lfs.github.com/spec/v1\n'):
        raise ValueError('binary or LFS pointer source unsupported')
    lines = blob.splitlines(keepends=True)
    if end > len(lines):
        raise ValueError('source range out of bounds')
    return b''.join(lines[start - 1:end])


def local_trial(root):
    upstream, fork, checkout = (root / name for name in ('upstream', 'fork', 'checkout'))
    git(root, 'init', '-q', '--initial-branch=main', str(upstream))
    git(upstream, 'config', 'user.name', 'Capability fixture')
    git(upstream, 'config', 'user.email', 'fixture@example.invalid')
    def commit(repo, message):
        git(repo, 'add', '-A')
        git(repo, '-c', 'commit.gpgsign=false', 'commit', '-qm', message)
        return git(repo, 'rev-parse', 'HEAD').decode().strip()
    (upstream / 'edit.txt').write_bytes(b'one\nold\nthree\n' + b'unchanged\n' * 12)
    (upstream / 'rename.txt').write_bytes(b'rename content\n' * 8)
    (upstream / 'deleted.txt').write_bytes(b'delete me\n')
    (upstream / 'crlf.txt').write_bytes(b'old\r\nline\r\n')
    (upstream / 'no-newline.txt').write_bytes(b'old')
    (upstream / 'binary.bin').write_bytes(b'\0old')
    initial = commit(upstream, 'common ancestor')
    git(root, 'clone', '-q', str(upstream), str(fork))
    git(root, 'clone', '-q', str(upstream), str(checkout))
    git(fork, 'config', 'user.name', 'Capability fixture')
    git(fork, 'config', 'user.email', 'fixture@example.invalid')
    (fork / 'edit.txt').write_bytes(b'one\nnew\nthree\n' + b'unchanged\n' * 12)
    (fork / 'rename.txt').rename(fork / 'renamed.txt')
    (fork / 'deleted.txt').unlink()
    (fork / 'added.txt').write_bytes(b'new file\n')
    (fork / 'crlf.txt').write_bytes(b'new\r\nline\r\n')
    (fork / 'no-newline.txt').write_bytes(b'new')
    (fork / 'binary.bin').write_bytes(b'\0new')
    git(fork, 'add', '-A')
    git(fork, 'update-index', '--add', '--cacheinfo', f'160000,{initial},module')
    git(fork, '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fork head')
    head = git(fork, 'rev-parse', 'HEAD').decode().strip()
    # Model GitHub's base-repository PR ref exposing fork objects.
    fetch(upstream, fork, head)
    git(upstream, 'update-ref', 'refs/pull/7/head', head)
    (upstream / 'target-only.txt').write_text('target moved\n')
    base = commit(upstream, 'target moved independently')
    (checkout / 'edit.txt').write_bytes(b'staged\n')
    git(checkout, 'add', 'edit.txt')
    (checkout / 'edit.txt').write_bytes(b'dirty unstaged\r\n')
    (checkout / 'untracked').write_bytes(b'untracked\x00bytes')
    before = snapshot(checkout)
    missing = subprocess.run(['git', '-C', str(checkout), 'cat-file', '-e', head],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE).returncode != 0
    mb = acquire(checkout, upstream, base, head, 'refs/pull/7/head')
    success_snapshot = snapshot(checkout)
    integrity = {'success': before == success_snapshot}
    try:
        acquire(checkout, upstream, 'f' * 40, head)
        raise AssertionError('missing object unexpectedly acquired')
    except subprocess.CalledProcessError:
        pass
    failure_snapshot = snapshot(checkout)
    integrity['failure'] = before == failure_snapshot
    # A real blocked upload-pack process: timeout kills and waits for git.
    helper = root / 'blocked-upload-pack'
    helper.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n')
    helper.chmod(0o755)
    try:
        run(['git', '-C', str(checkout), 'fetch', '--no-write-fetch-head',
             '--no-auto-maintenance', '--no-tags', '--no-recurse-submodules',
             '--upload-pack=' + str(helper), str(upstream), 'refs/pull/7/head'], timeout=0.2)
        raise AssertionError('expected cancellation timeout')
    except subprocess.TimeoutExpired:
        pass
    cancelled_snapshot = snapshot(checkout)
    integrity['cancelled'] = before == cancelled_snapshot
    hook = root / 'untrusted-diff'
    hook.write_text('#!/bin/sh\ntouch "' + str(root / 'HOOK_RAN') + '"\nexit 1\n')
    hook.chmod(0o755)
    git(checkout, 'config', 'diff.external', str(hook))
    git(checkout, 'config', 'diff.fixture.textconv', str(hook))
    info = checkout / '.git/info/attributes'
    info.write_text('edit.txt diff=fixture\n')
    result = comparison(checkout, 'fixture.invalid/owner/repo#7', base, head)
    assert not (root / 'HOOK_RAN').exists(), 'untrusted diff helper executed'
    assert source(checkout, head, 'edit.txt', 15, 15) == b'unchanged\n'
    assert not eligible(result['files']['edit.txt'], 'new', 15, 15)
    assert source(checkout, mb, 'edit.txt', 2, 2) == b'old\n'
    for path in ('../edit.txt', '/edit.txt', 'missing.txt'):
        try:
            source(checkout, head, path, 1, 1)
            raise AssertionError('invalid path accepted')
        except ValueError:
            pass
    crlf = git(checkout, 'show', head + ':crlf.txt')
    no_newline = git(checkout, 'show', head + ':no-newline.txt')
    return {'integrity': integrity, 'snapshots': {'before': before, 'success': success_snapshot,
            'failure': failure_snapshot, 'cancelled': cancelled_snapshot},
            'missing_before': missing, 'base': base, 'head': head, 'merge_base': mb,
            'source_base': initial, 'context': result['context'],
            'deterministic': result == comparison(checkout, 'fixture.invalid/owner/repo#7', base, head),
            'crlf_preserved': crlf == b'new\r\nline\r\n',
            'missing_newline_preserved': no_newline == b'new', 'anchors': result['files']}


def discussions(query):
    connections = []
    def pages(kind, thread=None):
        nodes, seen, cursor, expected, error = [], set(), None, None, None
        try:
            while True:
                page = query(kind, cursor, thread)
                expected = page['totalCount']
                nodes.extend(page['nodes'])
                info = page['pageInfo']
                if not info['hasNextPage']:
                    break
                cursor = info['endCursor']
                if not cursor or cursor in seen:
                    raise ValueError('missing or repeated pagination cursor')
                seen.add(cursor)
        except (RuntimeError, ValueError, KeyError, TypeError) as exc:
            error = str(exc)
        ids = [node['id'] for node in nodes if node is not None]
        complete = error is None and len(ids) == expected and len(set(ids)) == len(ids)
        connections.append({'scope': kind + (':' + thread if thread else ''),
                            'expected': expected, 'fetched': len(ids),
                            'complete': complete, 'error': error})
        return [node for node in nodes if node is not None]
    threads = pages('threads')
    for thread in threads:
        thread['comments'] = pages('comments', thread['id'])
    return {'threads': threads, 'connections': connections,
            'complete': all(connection['complete'] for connection in connections),
            'consistency': 'paginated observation, not an atomic server snapshot'}


THREAD_FIELDS = '''id path diffSide startDiffSide line startLine originalLine originalStartLine
isResolved isOutdated viewerCanResolve viewerCanUnresolve viewerCanReply'''
COMMENT_FIELDS = '''id databaseId body url author { login } replyTo { id databaseId }
line startLine originalLine originalStartLine diffHunk
viewerCanUpdate viewerCanDelete viewerDidAuthor createdAt updatedAt'''
PAGE = 'totalCount pageInfo { hasNextPage endCursor }'


def graphql(host, query, variables):
    payload = json.dumps({'query': query, 'variables': variables}).encode()
    try:
        response = json.loads(run(['gh', 'api', '--hostname', host, 'graphql', '--input', '-'],
                                  data=payload))
    except subprocess.CalledProcessError as exc:
        # Do not persist raw stderr, which can contain credentials/server details.
        raise RuntimeError(f'gh GraphQL request failed (exit {exc.returncode}); auth/access/network/schema not inferred') from exc
    if response.get('errors'):
        raise RuntimeError('GraphQL errors: ' + json.dumps(response['errors']))
    return response['data']


def live_trial(host, owner, name, number, page_size):
    metadata_query = '''query($owner:String!,$name:String!,$number:Int!) {
      repository(owner:$owner,name:$name) { pullRequest(number:$number) {
        id url baseRefOid headRefOid isCrossRepository viewerCanUpdate
        baseRepository { nameWithOwner } headRepository { nameWithOwner }
      } } }'''
    variables = {'owner': owner, 'name': name, 'number': number}
    metadata = graphql(host, metadata_query, variables)['repository']['pullRequest']
    def query(kind, cursor, thread=None):
        if kind == 'threads':
            document = ('query($owner:String!,$name:String!,$number:Int!,$cursor:String,$size:Int!) {'
                        'repository(owner:$owner,name:$name) { pullRequest(number:$number) {'
                        'reviewThreads(first:$size,after:$cursor) {' + PAGE + ' nodes {' + THREAD_FIELDS + '}}}}}')
            data = graphql(host, document, {**variables, 'cursor': cursor, 'size': page_size})
            return data['repository']['pullRequest']['reviewThreads']
        document = ('query($id:ID!,$cursor:String,$size:Int!) {node(id:$id) {'
                    '... on PullRequestReviewThread {comments(first:$size,after:$cursor) {'
                    + PAGE + ' nodes {' + COMMENT_FIELDS + '}}}}}')
        return graphql(host, document, {'id': thread, 'cursor': cursor, 'size': page_size})['node']['comments']
    discussion = discussions(query)
    with tempfile.TemporaryDirectory(prefix='bodh-live-pr-') as directory:
        root = Path(directory)
        git(root, 'init', '-q', '--initial-branch=trial')
        git(root, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
            '-c', 'commit.gpgsign=false', 'commit', '-qm', 'local checkout', '--allow-empty')
        (root / 'dirty').write_bytes(b'local dirty contents\r\n')
        before = snapshot(root)
        merge_base = acquire(root, f'https://{host}/{owner}/{name}.git',
                             metadata['baseRefOid'], metadata['headRefOid'], f'refs/pull/{number}/head')
        comparison_result = comparison(root, f'{host.lower()}/{owner.lower()}/{name.lower()}#{number}',
                                       metadata['baseRefOid'], metadata['headRefOid'])
        after = snapshot(root)
    return {'fixture': f'{host}/{owner}/{name}#{number}', 'metadata': metadata,
            'merge_base': merge_base, 'comparison': comparison_result, 'discussions': discussion,
            'integrity': before == after, 'snapshots': {'before': before, 'after': after}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', metavar='HOST/OWNER/REPO/NUMBER')
    parser.add_argument('--page-size', type=int, default=1, choices=range(1, 101))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = {'versions': {'git': run(['git', '--version']).decode().strip(),
                           'gh': run(['gh', '--version']).decode().splitlines()[0],
                           'python': platform.python_version(), 'system': platform.platform()},
              'revision': run(['git', 'rev-parse', 'HEAD']).decode().strip(),
              'dirty': bool(run(['git', 'status', '--porcelain'])),
              'host': 'local only', 'live_publication_tested': False}
    with tempfile.TemporaryDirectory(prefix='bodh-capability-') as directory:
        report['local'] = local_trial(Path(directory))
    if args.live:
        host, owner, name, number = args.live.split('/')
        if not re.fullmatch(r'[A-Za-z0-9.-]+', host) or not all(re.fullmatch(r'[A-Za-z0-9_.-]+', s) for s in (owner, name)):
            parser.error('invalid explicit repository identity')
        report['host'] = host
        report['live'] = live_trial(host, owner, name, int(number), args.page_size)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    if not all(report['local']['integrity'].values()):
        return 1
    if 'live' in report and not (report['live']['integrity'] and report['live']['discussions']['complete']):
        return 1
    print(f'Capability evidence: {args.output}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
