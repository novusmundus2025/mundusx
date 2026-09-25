import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from datetime import datetime, timedelta, timezone
import urllib.error

import artifact_upload as upload


class UploadTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.file = Path(self.directory.name)/'output.png'
        self.file.write_bytes(b'png-fixture')
        self.ticket_file = Path(self.directory.name)/'ticket.json'
        self.id = '12345678-1234-4123-8123-123456789abc'
        self.ticket = {'version':1, 'artifact_id':self.id, 'upload_path':f'/api/media/artifacts/{self.id}/content',
                       'upload_token':'a'*64, 'expires_at':(datetime.now(timezone.utc)+timedelta(hours=1)).isoformat(),
                       'byte_size':self.file.stat().st_size, 'sha256':hashlib.sha256(self.file.read_bytes()).hexdigest()}
        self.receipt = {'artifact_id':self.id, 'status':'ready', 'byte_size':self.ticket['byte_size'], 'sha256':self.ticket['sha256']}

    def save(self): self.ticket_file.write_text(json.dumps({'upload_ticket':self.ticket}))

    def test_upload_sends_scoped_token_and_does_not_delete_local_image(self):
        self.save()
        class Opener:
            def open(inner, request, timeout):
                self.assertEqual(request.method,'PUT')
                self.assertEqual(request.headers['Authorization'],'Bearer '+'a'*64)
                self.assertEqual(request.data,self.file.read_bytes())
                return io.BytesIO(json.dumps(self.receipt).encode())
        with patch.object(upload.urllib.request,'build_opener',return_value=Opener()):
            result=upload.upload_file(self.file,self.ticket_file,'https://images.example')
        self.assertEqual(result['status'],'ready'); self.assertTrue(self.file.exists())
        self.assertNotIn('upload_token',result)

    def test_wrong_file_expired_ticket_and_path_injection_never_send(self):
        for changed in [{'sha256':'0'*64}, {'expires_at':'2000-01-01T00:00:00Z'}, {'upload_path':'https://evil.example/upload'}]:
            original=dict(self.ticket); self.ticket.update(changed); self.save()
            with patch.object(upload.urllib.request,'build_opener') as network:
                with self.assertRaises(ValueError): upload.upload_file(self.file,self.ticket_file,'https://images.example')
                network.assert_not_called()
            self.ticket=original

    def test_remote_http_and_credential_urls_are_rejected(self):
        self.save()
        for server in ['http://images.example','https://user:secret@images.example','https://images.example?token=x']:
            with self.assertRaises(ValueError): upload.upload_file(self.file,self.ticket_file,server)

    def test_failed_or_invalid_receipt_preserves_file_and_hides_credentials(self):
        self.save()
        class Opener:
            def open(inner, request, timeout):
                raise urllib.error.HTTPError(request.full_url,403,'secret-token',None,None)
        with patch.object(upload.urllib.request,'build_opener',return_value=Opener()):
            with self.assertRaisesRegex(ValueError,'HTTP 403') as error:
                upload.upload_file(self.file,self.ticket_file,'https://images.example')
            self.assertNotIn('secret-token',str(error.exception))
        self.assertTrue(self.file.exists())


if __name__ == '__main__': unittest.main()
