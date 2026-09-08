package proxy

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const (
	fixtureChatDefault = "gemini-3.8-flash"
	fixtureImageGemini = "ag/gemini-3.1-flash-image"
	fixtureImageGPT    = "cx/gpt-5.5-image"
	fixtureImageApprox = "cx/gpt-5.6-image"
	fixtureNineRouter  = "6fcd27337a7893642c7fe630840d0a641743f28f"
	pngMagic           = "\x89PNG\r\n\x1a\n"
)

func testdataFile(t *testing.T, name string) []byte {
	t.Helper()
	body, err := os.ReadFile(filepath.Join("testdata", name))
	if err != nil {
		t.Fatal(err)
	}
	return body
}

func TestWorkbenchFixturesMatchFrozenNineRouterContract(t *testing.T) {
	var meta struct {
		Version string   `json:"version"`
		Commit  string   `json:"commit"`
		Notes   []string `json:"notes"`
	}
	if err := json.Unmarshal(testdataFile(t, "metadata.json"), &meta); err != nil {
		t.Fatal(err)
	}
	if meta.Version != "v0.5.45" || meta.Commit != fixtureNineRouter {
		t.Fatalf("metadata = %+v", meta)
	}
	if !strings.Contains(strings.Join(meta.Notes, "\n"), "URL-only") {
		t.Fatal("metadata must record that URL-only is not success")
	}

	chatIDs := catalogIDs(t, testdataFile(t, "models.json"))
	if !contains(chatIDs, fixtureChatDefault) {
		t.Fatal("chat fixture missing gemini-3.8-flash")
	}
	var chatHasSlash bool
	for _, id := range chatIDs {
		if strings.Contains(id, "/") {
			chatHasSlash = true
			break
		}
	}
	if !chatHasSlash {
		t.Fatal("chat fixture must include slash IDs so proxy transparency is observable")
	}
	simplified := simplifyChatIDs(chatIDs)
	for _, id := range simplified {
		if strings.Contains(id, "/") {
			t.Fatalf("simplified chat ID %q contains /", id)
		}
	}

	imageIDs := catalogIDs(t, testdataFile(t, "models_image.json"))
	if !contains(imageIDs, fixtureImageGemini) || !contains(imageIDs, fixtureImageGPT) {
		t.Fatalf("image fixture missing presets: %v", imageIDs)
	}
	if !contains(imageIDs, fixtureImageApprox) {
		t.Fatal("image fixture must include approximate GPT ID")
	}
	for _, id := range []string{fixtureImageGemini, fixtureImageGPT, fixtureImageApprox} {
		if !strings.Contains(id, "/") {
			t.Fatalf("image ID %q must retain /", id)
		}
	}
	if fixtureImageApprox == fixtureImageGPT {
		t.Fatal("approximate ID must not equal the gpt-image-2 preset")
	}

	sse := string(testdataFile(t, "chat_sse_with_image_block.txt"))
	if !strings.Contains(sse, "```image") || !strings.Contains(sse, `\"action\":\"generate\"`) {
		t.Fatal("SSE fixture must contain an image JSON fence")
	}

	png := testdataFile(t, "generation_binary.png")
	if !bytes.HasPrefix(png, []byte(pngMagic)) {
		t.Fatal("binary fixture is not PNG")
	}

	var b64 struct {
		Data []struct {
			B64JSON string `json:"b64_json"`
			URL     string `json:"url"`
		} `json:"data"`
	}
	if err := json.Unmarshal(testdataFile(t, "generation_b64_json.json"), &b64); err != nil {
		t.Fatal(err)
	}
	if len(b64.Data) != 1 || b64.Data[0].B64JSON == "" || b64.Data[0].URL != "" {
		t.Fatalf("b64 fixture = %+v", b64)
	}
	decoded, err := base64.StdEncoding.DecodeString(b64.Data[0].B64JSON)
	if err != nil || !bytes.Equal(decoded, png) {
		t.Fatal("b64_json must decode to the same PNG fixture")
	}

	var urlOnly struct {
		Data []struct {
			B64JSON string `json:"b64_json"`
			URL     string `json:"url"`
		} `json:"data"`
	}
	if err := json.Unmarshal(testdataFile(t, "generation_url_only.json"), &urlOnly); err != nil {
		t.Fatal(err)
	}
	if len(urlOnly.Data) != 1 || urlOnly.Data[0].URL == "" || urlOnly.Data[0].B64JSON != "" {
		t.Fatal("URL-only fixture must not be treated as a successful image payload")
	}
}

func catalogIDs(t *testing.T, body []byte) []string {
	t.Helper()
	var catalog struct {
		Data []struct {
			ID string `json:"id"`
		} `json:"data"`
	}
	if err := json.Unmarshal(body, &catalog); err != nil {
		t.Fatal(err)
	}
	ids := make([]string, 0, len(catalog.Data))
	for _, item := range catalog.Data {
		ids = append(ids, item.ID)
	}
	return ids
}

func simplifyChatIDs(ids []string) []string {
	out := make([]string, 0, len(ids))
	for _, id := range ids {
		if !strings.Contains(id, "/") {
			out = append(out, id)
		}
	}
	return out
}

func contains(ids []string, want string) bool {
	for _, id := range ids {
		if id == want {
			return true
		}
	}
	return false
}
