// Package spotcore calls the shared Rust session worker through its C interface.
package spotcore

/*
#cgo CFLAGS: -I${SRCDIR}/../c
#cgo LDFLAGS: -lspotcore_ffi
#include "spotcore.h"
*/
import "C"

import (
	"encoding/json"
	"errors"
	"sync"
	"unicode/utf8"
	"unsafe"
)

type Error struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

func (e *Error) Error() string { return e.Code + ": " + e.Message }

type SessionOptions struct {
	AppID      string  `json:"app_id"`
	DeviceName string  `json:"device_name"`
	Playback   bool    `json:"playback"`
	DataDir    *string `json:"data_dir"`
}
type Account struct {
	Username string  `json:"username"`
	Name     string  `json:"name"`
	Tier     string  `json:"tier"`
	Country  *string `json:"country"`
}
type Media struct {
	URI        string   `json:"uri"`
	Name       string   `json:"name"`
	ImageURLs  []string `json:"image_urls"`
	DurationMS *uint64  `json:"duration_ms"`
}
type SearchPage struct {
	Items      []Media `json:"items"`
	Total      *uint64 `json:"total"`
	NextOffset *uint32 `json:"next_offset"`
}
type SessionInfo struct {
	Account        Account `json:"account"`
	Engine         string  `json:"engine"`
	FallbackReason *string `json:"fallback_reason"`
}
type PlaybackUpdate struct {
	Kind       string  `json:"kind"`
	State      *string `json:"state"`
	Item       *Media  `json:"item"`
	PositionMS *uint64 `json:"position_ms"`
}

// Session must not be copied. Call Close before discarding it.
type Session struct {
	mu     sync.Mutex
	handle *C.spotcore_session
}

func NewSession() (*Session, error) {
	var output *C.char
	handle := C.spotcore_session_new(&output)
	if output != nil {
		defer C.spotcore_string_free(output)
		return nil, decode([]byte(C.GoString(output)), nil)
	}
	if handle == nil {
		return nil, errors.New("SpotCore returned no session")
	}
	return &Session{handle: handle}, nil
}
func decode(data []byte, result any) error {
	var envelope struct {
		OK     bool            `json:"ok"`
		Result json.RawMessage `json:"result"`
		Error  *Error          `json:"error"`
	}
	if err := json.Unmarshal(data, &envelope); err != nil {
		return err
	}
	if !envelope.OK {
		if envelope.Error != nil {
			return envelope.Error
		}
		return errors.New("Invalid SpotCore error response")
	}
	if result != nil {
		return json.Unmarshal(envelope.Result, result)
	}
	return nil
}
func (s *Session) callLocked(request any, result any) error {
	if s.handle == nil {
		return &Error{Code: "closed", Message: "Session is closed"}
	}
	data, err := json.Marshal(request)
	if err != nil {
		return err
	}
	response := C.spotcore_session_call(s.handle, (*C.uint8_t)(unsafe.Pointer(&data[0])), C.size_t(len(data)))
	if response == nil {
		return errors.New("SpotCore returned no response")
	}
	defer C.spotcore_string_free(response)
	return decode([]byte(C.GoString(response)), result)
}
func (s *Session) call(request any, result any) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.callLocked(request, result)
}
func valid(values ...string) error {
	for _, value := range values {
		if !utf8.ValidString(value) {
			return &Error{Code: "invalid_input", Message: "Input must be UTF-8"}
		}
	}
	return nil
}
func (s *Session) Login(cookie string, options SessionOptions, nativeKey *string) (SessionInfo, error) {
	var result SessionInfo
	values := []string{cookie, options.AppID, options.DeviceName}
	if options.DataDir != nil {
		values = append(values, *options.DataDir)
	}
	if nativeKey != nil {
		values = append(values, *nativeKey)
	}
	if err := valid(values...); err != nil {
		return result, err
	}
	err := s.call(map[string]any{"op": "login", "cookie": cookie, "options": options, "native_key": nativeKey}, &result)
	return result, err
}
func (s *Session) Account() (Account, error) {
	var result Account
	err := s.call(map[string]any{"op": "account"}, &result)
	return result, err
}
func (s *Session) SearchTracks(query string, limit, offset uint32) (SearchPage, error) {
	var result SearchPage
	if err := valid(query); err != nil {
		return result, err
	}
	err := s.call(map[string]any{"op": "search_tracks", "query": query, "limit": limit, "offset": offset}, &result)
	return result, err
}
func (s *Session) Play(uri string) error {
	if err := valid(uri); err != nil {
		return err
	}
	return s.call(map[string]any{"op": "play", "uri": uri}, nil)
}
func (s *Session) Pause() error { return s.call(map[string]any{"op": "pause"}, nil) }
func (s *Session) PollEvent() (*PlaybackUpdate, error) {
	var result *PlaybackUpdate
	err := s.call(map[string]any{"op": "poll_event"}, &result)
	return result, err
}
func (s *Session) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.handle == nil {
		return nil
	}
	err := s.callLocked(map[string]any{"op": "shutdown"}, nil)
	C.spotcore_session_free(s.handle)
	s.handle = nil
	return err
}

// Invoke calls an operation from the shared SDK inventory. Results retain SDK JSON fields.
func (s *Session) Invoke(operation string, arguments map[string]any) (json.RawMessage, error) {
	if err := valid(operation); err != nil {
		return nil, err
	}
	if arguments == nil {
		arguments = map[string]any{}
	}
	var result json.RawMessage
	err := s.call(map[string]any{"op": "invoke", "operation": operation, "arguments": arguments}, &result)
	return result, err
}

type OperationInfo struct {
	Result    string            `json:"result"`
	Operation string            `json:"operation"`
	Arguments map[string]string `json:"arguments"`
}

func (s *Session) Operations() ([]OperationInfo, error) {
	var result []OperationInfo
	err := s.call(map[string]any{"op": "operations"}, &result)
	return result, err
}
