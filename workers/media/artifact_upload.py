"""Scoped media upload. No account password, operator token or storage key is sent."""
import hashlib
import json
import os
import stat
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

MAX_BYTES = 128 * 1024 * 1024


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def owned_file(file, directory):
    """Only direct regular files in a fixed application directory may leave the host."""
    path = Path(os.path.abspath(file))
    directory = Path(os.path.abspath(directory))
    if path.parent != directory or directory.resolve() != directory:
        raise ValueError('File is outside the owned media directory')
    metadata = path.lstat()
    if (path.is_symlink() or not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 or
            getattr(metadata, 'st_file_attributes', 0) & 0x400):
        raise ValueError('Media file must be a regular file without links')
    return path


def read_owned(file, directory, maximum):
    path = owned_file(file, directory)
    before = path.lstat()
    descriptor = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_BINARY', 0))
    with os.fdopen(descriptor, 'rb') as stream:
        opened = os.fstat(stream.fileno())
        if (not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1 or
                (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)):
            raise ValueError('Media file changed while opening')
        return stream.read(maximum + 1)


def upload_file(file, ticket_path, server, managed_copy=None, *, media_root):
    if not file or not ticket_path or not server:
        raise ValueError('Upload requires an image file, upload ticket and web server URL')
    origin = urllib.parse.urlsplit(server)
    if (origin.scheme not in ('http', 'https') or not origin.hostname or origin.username or origin.password or
            origin.query or origin.fragment or origin.path not in ('', '/') or
            (origin.scheme == 'http' and origin.hostname not in ('localhost', '127.0.0.1', '::1'))):
        raise ValueError('Use an HTTPS web-server origin (HTTP is allowed only on loopback)')
    root = Path(os.path.abspath(media_root))
    ticket_data = read_owned(ticket_path, root, 16 * 1024)
    if len(ticket_data) > 16 * 1024:
        raise ValueError('Upload ticket is too large')
    ticket = json.loads(ticket_data)
    ticket = ticket.get('upload_ticket', ticket)
    identity = ticket.get('artifact_id', '')
    if not re.fullmatch(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', identity):
        raise ValueError('Invalid artifact identifier')
    expected_path = '/api/media/artifacts/' + identity + '/content'
    if ticket.get('version') != 1 or ticket.get('upload_path') != expected_path or not re.fullmatch(r'[0-9a-f]{64}', ticket.get('upload_token', '')):
        raise ValueError('Invalid scoped upload ticket')
    if datetime.fromisoformat(ticket['expires_at'].replace('Z', '+00:00')) <= datetime.now(timezone.utc):
        raise ValueError('Upload ticket expired; request a new reservation')
    content_type = ticket.get('content_type', 'image/png')
    if content_type not in ('image/png', 'video/mp4'):
        raise ValueError('Unsupported artifact content type')
    maximum = MAX_BYTES if content_type == 'video/mp4' else 32 * 1024 * 1024
    path = owned_file(file, root / 'artifacts')
    if not re.fullmatch(r'[0-9a-f]{32}\.(png|mp4)', path.name):
        raise ValueError('Invalid generated artifact name')
    if managed_copy is not None:
        managed_copy = owned_file(managed_copy, root / 'outputs' / 'opengpu')
    data = read_owned(path, root / 'artifacts', maximum)
    digest = hashlib.sha256(data).hexdigest()
    if not 0 < len(data) <= maximum or len(data) != ticket.get('byte_size') or digest != ticket.get('sha256'):
        raise ValueError('Image does not match the upload ticket size and checksum')
    request = urllib.request.Request(server.rstrip('/') + expected_path, data=data, method='PUT', headers={
        'Authorization': 'Bearer ' + ticket['upload_token'], 'Content-Type': content_type,
        'Content-Length': str(len(data)),
    })
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=120) as response:
            result = response.read(16 * 1024 + 1)
            if len(result) > 16 * 1024: raise ValueError('Upload receipt is too large')
            receipt = json.loads(result)
    except urllib.error.HTTPError as error:
        raise ValueError(f'Image upload rejected (HTTP {error.code}); local image preserved') from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise ValueError('Image upload connection failed; local image preserved; retry with the same ticket') from None
    if (receipt.get('artifact_id') != identity or receipt.get('status') != 'ready' or
            receipt.get('sha256') != digest or receipt.get('byte_size') != len(data)):
        raise ValueError('Invalid upload receipt; local image preserved')
    deleted = False
    if content_type == 'image/png':
        paths = [path]
        if managed_copy is not None and Path(managed_copy) != path:
            paths.insert(0, Path(managed_copy))
        # Revalidate all copies before deletion: a changed file is not this upload.
        for candidate in paths:
            if not candidate.exists():
                if candidate == path:
                    raise ValueError('Hosted image is ready, but the local image disappeared before cleanup')
                continue
            if candidate.is_symlink() or not candidate.is_file():
                raise ValueError('Hosted image is ready; refusing to delete a non-regular local copy')
            current = read_owned(candidate, root / ('artifacts' if candidate == path else 'outputs/opengpu'), maximum)
            if len(current) != len(data) or hashlib.sha256(current).hexdigest() != digest:
                raise ValueError('Hosted image is ready; local copy changed, so cleanup was refused')
        try:
            for candidate in paths:
                candidate.unlink(missing_ok=True)  # Direct filesystem deletion; no Trash/Recycle Bin.
        except OSError as error:
            raise ValueError('Hosted image is ready, but permanent local cleanup failed; check file permissions') from error
        deleted = True
    # The requesting user already owns the download link. Never expose the upload secret.
    return {'artifact_id': identity, 'status': 'ready', 'sha256': digest,
            'byte_size': len(data), 'expires_at': receipt.get('expires_at'), 'local_image_preserved': not deleted, 'local_image_deleted': deleted}
