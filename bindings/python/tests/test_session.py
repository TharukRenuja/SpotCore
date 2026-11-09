import asyncio
import json
import unittest

from spotcore import BindingErrorException, Session, SessionOptions


class SessionTests(unittest.IsolatedAsyncioTestCase):
    async def test_signed_out_calls_and_shutdown(self):
        session = Session()
        try:
            inventory = json.loads(session.operations())
            self.assertTrue(any(op['operation'] == 'library.add' for op in inventory))
            with self.assertRaises(BindingErrorException) as caught:
                await session.invoke('library.contains', '{"values":["spotify:track:test"]}')
            self.assertEqual(caught.exception.error.code, 'not_authenticated')
            results = await asyncio.gather(
                session.account(), session.search_tracks("Miles Davis", 5, 0),
                session.next_event(), return_exceptions=True,
            )
            for error in results:
                self.assertIsInstance(error, BindingErrorException)
                self.assertEqual(error.error.code, "not_authenticated")
            options = SessionOptions(
                app_id="spotcore-binding-test", device_name="Binding test",
                playback=False, data_dir=None,
            )
            with self.assertRaises(BindingErrorException) as caught:
                await session.login("", options, None)
            self.assertEqual(caught.exception.error.code, "invalid_input")
        finally:
            await session.shutdown()
        with self.assertRaises(BindingErrorException) as caught:
            await session.account()
        self.assertEqual(caught.exception.error.code, "closed")


if __name__ == "__main__":
    unittest.main()
