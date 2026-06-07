---
name: '⚠️ General Problem / Chore'
about: 'Report architectural concern, performance drain, or general maintenance need.'
title: '[CHORE] General system concern area affected by X'
ref: 'main'
labels:
  - chore
  - help needed
---
**Problem Description**

Describe systemic issue. Not specific bug. System failing to meet non-functional requirement (e.g., performance, scalability, security).

**Scope & Impact**
*   Where does problem manifest? (Module/Service name)
*   How bad is it? (Low/Medium/High impact; affects N users/requests per second).
*   Impact on current roadmap: [Yes/No].

**Observed Behavior vs. Expected State**
*   Current state: System operates slowly/fails under load Z, using method Y.
*   Desired state: Requirement needs re-evaluation. Target behavior must achieve [metric] improvement.

**Hypothesis / Suggested Direction**
Initial thought for fix or refactor direction (optional). e.g., Investigate moving from async runtime A to B.

**Required Stakeholders**
List engineers who need review/input before solution implemented.
