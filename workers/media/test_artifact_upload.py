import hashlib
import io
import json
import os
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

    def test_upload_sends_scoped_token_then_permanently_deletes_local_image(self):
        self.save()
        class Opener:
            def open(inner, request, timeout):
                self.assertEqual(request.method,'PUT')
                self.assertEqual(request.headers['Authorization'],'Bearer '+'a'*64)
                self.assertEqual(request.data,self.file.read_bytes())
                return io.BytesIO(json.dumps(self.receipt).encode())
        with patch.object(upload.urllib.request,'build_opener',return_value=Opener()):
            result=upload.upload_file(self.file,self.ticket_file,'https://images.example')
        self.assertEqual(result['status'],'ready'); self.assertFalse(self.file.exists())
        self.assertTrue(result['local_image_deleted']); self.assertFalse(result['local_image_preserved'])
        self.assertNotIn('upload_token',result)

    def test_open_plane_upload_needs_no_gateway_token(self):
        self.save()
        class Opener:
            def open(inner, request, timeout):
                self.assertIsNone(request.get_header('Coder-session-token'))
                self.assertEqual(request.headers['Authorization'],'Bearer '+'a'*64)
                return io.BytesIO(json.dumps(self.receipt).encode())
        with patch.dict(os.environ, {}, clear=True), \
             patch.object(upload.urllib.request,'build_opener',return_value=Opener()):
            self.assertEqual(upload.upload_file(self.file,self.ticket_file,'https://images.example')['status'],'ready')

    def test_private_gateway_header_preserves_artifact_bearer(self):
        self.save()
        auth = json.dumps({'url':'https://images.example'+self.ticket['upload_path'],
                           'header':'Coder-Session-Token','token':'gateway-fixture'})
        class Opener:
            def open(inner, request, timeout):
                self.assertEqual(request.headers['Authorization'],'Bearer '+'a'*64)
                self.assertEqual(request.get_header('Coder-session-token'),'gateway-fixture')
                return io.BytesIO(json.dumps(self.receipt).encode())
        with patch.dict(os.environ, {'OPENGPU_MEDIA_GATEWAY_AUTH':auth}), \
             patch.object(upload.urllib.request,'build_opener',return_value=Opener()):
            result=upload.upload_file(self.file,self.ticket_file,'https://images.example')
        self.assertNotIn('gateway-fixture',json.dumps(result))

    def test_gateway_credentials_never_go_to_another_upload_or_header(self):
        self.save()
        for changed in [{'url':'https://other.example'+self.ticket['upload_path']},
                        {'url':'https://images.example/api/other'},
                        {'header':'Authorization'}, {'token':'bad\nheader'}]:
            auth={'url':'https://images.example'+self.ticket['upload_path'],
                  'header':'Coder-Session-Token','token':'gateway-fixture',**changed}
            with patch.dict(os.environ, {'OPENGPU_MEDIA_GATEWAY_AUTH':json.dumps(auth)}), \
                 patch.object(upload.urllib.request,'build_opener') as network:
                with self.assertRaises(ValueError) as error:
                    upload.upload_file(self.file,self.ticket_file,'https://images.example')
                self.assertNotIn('gateway-fixture',str(error.exception))
                network.assert_not_called()
            self.assertTrue(self.file.exists())

    def successful_opener(self, receipt=None, mutate=None):
        value = self.receipt if receipt is None else receipt
        class Opener:
            def open(inner, request, timeout):
                if mutate: mutate()
                return io.BytesIO(json.dumps(value).encode())
        return Opener()

    def test_invalid_receipt_never_deletes_local_image(self):
        self.save()
        for changed in [{'status':'pending'}, {'sha256':'0'*64}, {'byte_size':0}, {'artifact_id':'other'}]:
            with patch.object(upload.urllib.request,'build_opener',return_value=self.successful_opener({**self.receipt,**changed})):
                with self.assertRaisesRegex(ValueError,'Invalid upload receipt'):
                    upload.upload_file(self.file,self.ticket_file,'https://images.example')
            self.assertTrue(self.file.exists())

    def test_managed_original_and_upload_copy_are_both_deleted(self):
        self.save()
        original = Path(self.directory.name)/'comfy-output.png'
        original.write_bytes(self.file.read_bytes())
        with patch.object(upload.urllib.request,'build_opener',return_value=self.successful_opener()):
            upload.upload_file(self.file,self.ticket_file,'https://images.example',original)
        self.assertFalse(original.exists()); self.assertFalse(self.file.exists())

    def test_changed_managed_output_is_preserved_and_no_copy_deleted(self):
        self.save()
        original = Path(self.directory.name)/'comfy-output.png'
        original.write_bytes(b'other-image')
        with patch.object(upload.urllib.request,'build_opener',return_value=self.successful_opener()):
            with self.assertRaisesRegex(ValueError,'local copy changed'):
                upload.upload_file(self.file,self.ticket_file,'https://images.example',original)
        self.assertTrue(original.exists()); self.assertTrue(self.file.exists())

    def test_cleanup_failure_is_reported_not_silently_ignored(self):
        self.save()
        with patch.object(upload.urllib.request,'build_opener',return_value=self.successful_opener()), \
             patch.object(Path,'unlink',side_effect=PermissionError('denied')):
            with self.assertRaisesRegex(ValueError,'permanent local cleanup failed'):
                upload.upload_file(self.file,self.ticket_file,'https://images.example')
        self.assertTrue(self.file.exists())

    def test_video_retention_is_unchanged(self):
        self.ticket['content_type']='video/mp4'; self.save()
        with patch.object(upload.urllib.request,'build_opener',return_value=self.successful_opener()):
            result=upload.upload_file(self.file,self.ticket_file,'https://images.example')
        self.assertTrue(self.file.exists()); self.assertFalse(result['local_image_deleted'])

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
