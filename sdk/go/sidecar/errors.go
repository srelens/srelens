package sidecar

import "errors"

var (
	// ErrProtocol: srelens wrote a line that is not JSON-RPC, which ends the
	// session.
	ErrProtocol = errors.New("srelens wrote a line that is not JSON-RPC")
	// ErrSessionEnded: the session with srelens ended. It is a handler's
	// context's cause at shutdown or the end of input, a host call's error
	// when the session ends before its answer, and Frames.Send's error after
	// the end.
	ErrSessionEnded = errors.New("the session with srelens ended")
)
