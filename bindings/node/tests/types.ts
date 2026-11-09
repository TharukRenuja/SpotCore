import { Session, SessionOptions, SessionInfo, SearchPage, PlaybackUpdate } from '..';
const session = new Session();
const options: SessionOptions = { appId: 'type-test', deviceName: 'Test', playback: false };
const info: Promise<SessionInfo> = session.login('not-a-real-cookie', options);
const search: Promise<SearchPage> = session.searchTracks('Miles Davis', 10, 0);
const event: Promise<PlaybackUpdate> = session.nextEvent();
void [info, search, event];

void session.invoke("library.contains", { values: ["spotify:track:test"] });
void session.operations();
