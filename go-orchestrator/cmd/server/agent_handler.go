package main

import (
	"context"
	"encoding/json"
	"log"
	"net/http"
	"net/http/httptest"
	"strings"
	"time"

	"optima/go-orchestrator/internal/agent"
)

type agentPlanRequest struct {
	Message string `json:"message"`
	Locale  string `json:"locale"`
}

// AgentPlanResponse is chat + structured panel + optional schedule payload.
type AgentPlanResponse struct {
	SchemaVersion          string                 `json:"schema_version"`
	ClarificationNeeded    bool                   `json:"clarification_needed"`
	ClarificationQuestions []string               `json:"clarification_questions,omitempty"`
	StructuredPanel        *agent.ScheduleIntentV1  `json:"structured_panel,omitempty"`
	Schedules              *ScheduleResponse      `json:"schedules,omitempty"`
	ScheduleQuery          string                 `json:"schedule_query,omitempty"`
	Error                  string                 `json:"error,omitempty"`
}

func agentPlanHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var body agentPlanRequest
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "invalid json", http.StatusBadRequest)
		return
	}
	if strings.TrimSpace(body.Message) == "" {
		http.Error(w, "message is required", http.StatusBadRequest)
		return
	}
	if len(body.Message) > 8000 {
		http.Error(w, "message too long", http.StatusBadRequest)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), 85*time.Second)
	defer cancel()

	system := agent.BuildSystemPrompt()
	user := agent.BuildUserMessage(body.Message, body.Locale)
	rawLLM, err := agent.CompleteJSON(ctx, system, user)
	if err != nil {
		log.Printf("agent openai: %v", err)
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusServiceUnavailable)
		_ = json.NewEncoder(w).Encode(AgentPlanResponse{
			SchemaVersion: agent.SchemaVersionScheduleIntentV1,
			Error:         err.Error(),
		})
		return
	}

	doc, err := agent.ParseAndValidate(rawLLM)
	if err != nil {
		http.Error(w, "invalid agent output: "+err.Error(), http.StatusBadGateway)
		return
	}

	out := AgentPlanResponse{
		SchemaVersion:          doc.SchemaVersion,
		ClarificationNeeded:    doc.ClarificationNeeded,
		ClarificationQuestions: doc.ClarificationQuestions,
		StructuredPanel:        doc,
	}

	if doc.ClarificationNeeded {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_ = json.NewEncoder(w).Encode(out)
		return
	}

	q := agent.IntentToScheduleQuery(doc.Intent)
	out.ScheduleQuery = q
	u := "/v1/schedules"
	if q != "" {
		u += "?" + q
	}
	req2 := httptest.NewRequest(http.MethodGet, u, nil)
	req2 = req2.WithContext(ctx)
	rec := httptest.NewRecorder()
	schedulesHandler(rec, req2)

	if rec.Code != http.StatusOK {
		http.Error(w, "schedule engine error: "+rec.Body.String(), rec.Code)
		return
	}
	var sched ScheduleResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &sched); err != nil {
		http.Error(w, "schedule decode: "+err.Error(), http.StatusInternalServerError)
		return
	}
	out.Schedules = &sched

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	if err := json.NewEncoder(w).Encode(out); err != nil {
		log.Printf("agent encode: %v", err)
	}
}
