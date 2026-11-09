package main

import (
	"fmt"
	spotcore "github.com/TharukRenuja/SpotCore/bindings/go"
	"os"
)

func main() {
	cookie, err := os.ReadFile(os.Getenv("SPOTCORE_COOKIE_FILE"))
	if err != nil {
		panic(err)
	}
	spotify, err := spotcore.NewSession()
	if err != nil {
		panic(err)
	}
	defer spotify.Close()
	_, err = spotify.Login(string(cookie), spotcore.SessionOptions{
		AppID: "my-go-app", DeviceName: "My Music App", Playback: false,
	}, nil)
	if err != nil {
		panic(err)
	}
	page, err := spotify.SearchTracks("Miles Davis", 10, 0)
	if err != nil {
		panic(err)
	}
	for _, track := range page.Items {
		fmt.Printf("%s — %s\n", track.Name, track.URI)
	}
}
