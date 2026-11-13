//! UI-independent collaboration policy. Authority is loaded by the server, never
//! deserialized from a browser/tool request. Legacy relay roles are not team roles.
use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
    Reviewer,
    Viewer,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ViewProject,
    ViewSharedSession,
    CreateSession,
    ShareOwnSession,
    EditWorkflow,
    RunWorkflow,
    Review,
    Publish,
    ManageProject,
    ManageHosts,
    ManageMembers,
    ManageProjects,
    TransferOwnership,
}

impl Permission {
    pub fn is_workspace_scoped(self) -> bool {
        matches!(
            self,
            Self::ManageHosts
                | Self::ManageMembers
                | Self::ManageProjects
                | Self::TransferOwnership
        )
    }
}

impl WorkspaceRole {
    pub fn permits(self, permission: Permission) -> bool {
        match self {
            Self::Owner => true,
            Self::Admin => permission != Permission::TransferOwnership,
            Self::Member => matches!(
                permission,
                Permission::ViewProject
                    | Permission::ViewSharedSession
                    | Permission::CreateSession
                    | Permission::ShareOwnSession
                    | Permission::EditWorkflow
                    | Permission::RunWorkflow
            ),
            Self::Reviewer => matches!(
                permission,
                Permission::ViewProject | Permission::ViewSharedSession | Permission::Review
            ),
            Self::Viewer => matches!(
                permission,
                Permission::ViewProject | Permission::ViewSharedSession
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Membership {
    pub workspace_id: String,
    pub user_id: String,
    pub role: WorkspaceRole,
    pub revision: u64,
    pub active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectGrant {
    pub workspace_id: String,
    pub project_id: String,
    pub user_id: String,
    pub permissions: BTreeSet<Permission>,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrincipalKind {
    Human,
    Service,
}

/// Not Deserialize: only authenticated server adapters construct this context.
#[derive(Clone, Debug)]
pub struct AccessContext {
    pub principal: PrincipalKind,
    pub membership: Membership,
    pub project_grant: Option<ProjectGrant>,
}

/// Approval scope includes both revisions, so role/grant changes invalidate it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorityStamp {
    pub workspace_id: String,
    pub project_id: String,
    pub user_id: String,
    pub membership_revision: u64,
    pub grant_revision: u64,
}

/// Session privacy is explicit and independent of workspace administration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSharePolicy {
    pub workspace_id: String,
    pub project_id: String,
    pub owner_id: String,
    pub reader_ids: BTreeSet<String>,
}

impl SessionSharePolicy {
    pub fn authorize_read(&self, context: &AccessContext) -> Result<AuthorityStamp, PolicyError> {
        let stamp = context.authorize_project(
            &self.workspace_id,
            &self.project_id,
            Permission::ViewProject,
        )?;
        if self.owner_id == context.membership.user_id {
            return Ok(stamp);
        }
        if !self.reader_ids.contains(&context.membership.user_id) {
            return Err(PolicyError::PermissionDenied);
        }
        context.authorize_project(
            &self.workspace_id,
            &self.project_id,
            Permission::ViewSharedSession,
        )
    }

    pub fn authorize_share(
        &self,
        owner: &AccessContext,
        recipient: &AccessContext,
    ) -> Result<AuthorityStamp, PolicyError> {
        if owner.principal != PrincipalKind::Human || recipient.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied);
        }
        if self.owner_id != owner.membership.user_id {
            return Err(PolicyError::PermissionDenied);
        }
        let stamp = owner.authorize_project(
            &self.workspace_id,
            &self.project_id,
            Permission::ShareOwnSession,
        )?;
        recipient.authorize_project(
            &self.workspace_id,
            &self.project_id,
            Permission::ViewSharedSession,
        )?;
        Ok(stamp)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    InactiveMembership,
    ScopeMismatch,
    PermissionDenied,
    ServiceCannotApprove,
    SelfApproval,
    StaleAuthority,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InactiveMembership => "Workspace membership is inactive or invalid",
            Self::ScopeMismatch => "Authority does not match the requested scope",
            Self::PermissionDenied => "Permission is not granted",
            Self::ServiceCannotApprove => "Service principals cannot make human approvals",
            Self::SelfApproval => "This operation requires another reviewer",
            Self::StaleAuthority => "Authority changed; review the operation again",
        })
    }
}
impl std::error::Error for PolicyError {}

impl AccessContext {
    fn active(&self, workspace: &str) -> Result<(), PolicyError> {
        if !self.membership.active
            || self.membership.revision == 0
            || self.membership.user_id.is_empty()
            || self.membership.workspace_id.is_empty()
        {
            return Err(PolicyError::InactiveMembership);
        }
        if self.membership.workspace_id != workspace {
            return Err(PolicyError::ScopeMismatch);
        }
        Ok(())
    }

    pub fn authorize_workspace(
        &self,
        workspace: &str,
        permission: Permission,
    ) -> Result<(), PolicyError> {
        self.active(workspace)?;
        if !permission.is_workspace_scoped() || !self.membership.role.permits(permission) {
            return Err(PolicyError::PermissionDenied);
        }
        Ok(())
    }

    pub fn authorize_project(
        &self,
        workspace: &str,
        project: &str,
        permission: Permission,
    ) -> Result<AuthorityStamp, PolicyError> {
        self.active(workspace)?;
        let grant = self
            .project_grant
            .as_ref()
            .ok_or(PolicyError::PermissionDenied)?;
        if grant.workspace_id != workspace
            || grant.project_id != project
            || grant.user_id != self.membership.user_id
            || grant.revision == 0
        {
            return Err(PolicyError::ScopeMismatch);
        }
        // Roles are a ceiling, not a substitute for an explicit project grant.
        if permission.is_workspace_scoped()
            || !self.membership.role.permits(permission)
            || !grant.permissions.contains(&permission)
        {
            return Err(PolicyError::PermissionDenied);
        }
        Ok(AuthorityStamp {
            workspace_id: workspace.into(),
            project_id: project.into(),
            user_id: self.membership.user_id.clone(),
            membership_revision: self.membership.revision,
            grant_revision: grant.revision,
        })
    }

    pub fn authorize_approval(
        &self,
        workspace: &str,
        project: &str,
        initiator: &str,
        separation_of_duties: bool,
    ) -> Result<AuthorityStamp, PolicyError> {
        if self.principal != PrincipalKind::Human {
            return Err(PolicyError::ServiceCannotApprove);
        }
        let stamp = self.authorize_project(workspace, project, Permission::Review)?;
        if separation_of_duties && self.membership.user_id == initiator {
            return Err(PolicyError::SelfApproval);
        }
        Ok(stamp)
    }

    pub fn revalidate(
        &self,
        stamp: &AuthorityStamp,
        permission: Permission,
    ) -> Result<(), PolicyError> {
        let current = self.authorize_project(&stamp.workspace_id, &stamp.project_id, permission)?;
        if current != *stamp {
            return Err(PolicyError::StaleAuthority);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(role: WorkspaceRole) -> AccessContext {
        AccessContext {
            principal: PrincipalKind::Human,
            membership: Membership {
                workspace_id: "w".into(),
                user_id: "u".into(),
                role,
                revision: 1,
                active: true,
            },
            project_grant: Some(ProjectGrant {
                workspace_id: "w".into(),
                project_id: "p".into(),
                user_id: "u".into(),
                revision: 1,
                permissions: [
                    Permission::ViewProject,
                    Permission::ViewSharedSession,
                    Permission::CreateSession,
                    Permission::ShareOwnSession,
                    Permission::EditWorkflow,
                    Permission::RunWorkflow,
                    Permission::Review,
                    Permission::Publish,
                ]
                .into(),
            }),
        }
    }

    #[test]
    fn grant_cannot_escalate_a_viewer_to_execution() {
        let viewer = context(WorkspaceRole::Viewer);
        assert!(
            viewer
                .authorize_project("w", "p", Permission::ViewProject)
                .is_ok()
        );
        assert_eq!(
            viewer.authorize_project("w", "p", Permission::RunWorkflow),
            Err(PolicyError::PermissionDenied)
        );
    }

    #[test]
    fn owner_still_needs_a_project_grant() {
        let mut owner = context(WorkspaceRole::Owner);
        owner.project_grant = None;
        assert_eq!(
            owner.authorize_project("w", "p", Permission::ViewProject),
            Err(PolicyError::PermissionDenied)
        );
        assert!(
            owner
                .authorize_workspace("w", Permission::ManageMembers)
                .is_ok()
        );
    }

    #[test]
    fn reject_cross_workspace_project_or_user_authority() {
        let mut member = context(WorkspaceRole::Member);
        assert_eq!(
            member.authorize_project("other", "p", Permission::ViewProject),
            Err(PolicyError::ScopeMismatch)
        );
        assert_eq!(
            member.authorize_project("w", "other", Permission::ViewProject),
            Err(PolicyError::ScopeMismatch)
        );
        member.project_grant.as_mut().unwrap().user_id = "someone-else".into();
        assert_eq!(
            member.authorize_project("w", "p", Permission::ViewProject),
            Err(PolicyError::ScopeMismatch)
        );
    }

    #[test]
    fn workspace_permissions_cannot_be_smuggled_through_project_grants() {
        let mut member = context(WorkspaceRole::Member);
        member
            .project_grant
            .as_mut()
            .unwrap()
            .permissions
            .insert(Permission::ManageMembers);
        assert_eq!(
            member.authorize_project("w", "p", Permission::ManageMembers),
            Err(PolicyError::PermissionDenied)
        );
    }

    #[test]
    fn service_cannot_approve_even_with_admin_role() {
        let mut admin = context(WorkspaceRole::Admin);
        admin.principal = PrincipalKind::Service;
        assert_eq!(
            admin.authorize_approval("w", "p", "someone-else", false),
            Err(PolicyError::ServiceCannotApprove)
        );
    }

    #[test]
    fn separation_of_duties_is_explicit_and_enforced() {
        let reviewer = context(WorkspaceRole::Reviewer);
        assert_eq!(
            reviewer.authorize_approval("w", "p", "u", true),
            Err(PolicyError::SelfApproval)
        );
        assert!(reviewer.authorize_approval("w", "p", "u", false).is_ok());
        assert!(reviewer.authorize_approval("w", "p", "other", true).is_ok());
    }

    #[test]
    fn authority_revision_or_membership_change_invalidates_approval() {
        let mut reviewer = context(WorkspaceRole::Reviewer);
        let stamp = reviewer
            .authorize_approval("w", "p", "other", true)
            .unwrap();
        reviewer.membership.revision += 1;
        assert_eq!(
            reviewer.revalidate(&stamp, Permission::Review),
            Err(PolicyError::StaleAuthority)
        );
        reviewer.membership.active = false;
        assert_eq!(
            reviewer.revalidate(&stamp, Permission::Review),
            Err(PolicyError::InactiveMembership)
        );
    }

    #[test]
    fn grant_revision_change_alone_requires_review_again() {
        let mut reviewer = context(WorkspaceRole::Reviewer);
        let stamp = reviewer
            .authorize_approval("w", "p", "other", true)
            .unwrap();
        reviewer.project_grant.as_mut().unwrap().revision += 1;
        assert_eq!(
            reviewer.revalidate(&stamp, Permission::Review),
            Err(PolicyError::StaleAuthority)
        );
    }

    #[test]
    fn ownership_transfer_is_not_an_admin_permission() {
        assert!(
            context(WorkspaceRole::Owner)
                .authorize_workspace("w", Permission::TransferOwnership)
                .is_ok()
        );
        assert_eq!(
            context(WorkspaceRole::Admin).authorize_workspace("w", Permission::TransferOwnership),
            Err(PolicyError::PermissionDenied)
        );
    }

    #[test]
    fn workspace_owner_does_not_inherit_another_users_private_transcript() {
        let policy = SessionSharePolicy {
            workspace_id: "w".into(),
            project_id: "p".into(),
            owner_id: "other".into(),
            reader_ids: BTreeSet::new(),
        };
        assert_eq!(
            policy.authorize_read(&context(WorkspaceRole::Owner)),
            Err(PolicyError::PermissionDenied)
        );
    }

    #[test]
    fn explicit_session_share_still_requires_current_project_authority() {
        let policy = SessionSharePolicy {
            workspace_id: "w".into(),
            project_id: "p".into(),
            owner_id: "other".into(),
            reader_ids: ["u".into()].into(),
        };
        let mut reader = context(WorkspaceRole::Viewer);
        assert!(policy.authorize_read(&reader).is_ok());
        reader.project_grant = None;
        assert_eq!(
            policy.authorize_read(&reader),
            Err(PolicyError::PermissionDenied)
        );
    }

    #[test]
    fn sharing_requires_owner_and_a_granted_recipient() {
        let policy = SessionSharePolicy {
            workspace_id: "w".into(),
            project_id: "p".into(),
            owner_id: "u".into(),
            reader_ids: BTreeSet::new(),
        };
        let owner = context(WorkspaceRole::Member);
        let mut recipient = context(WorkspaceRole::Viewer);
        recipient.membership.user_id = "reader".into();
        recipient.project_grant.as_mut().unwrap().user_id = "reader".into();
        assert!(policy.authorize_share(&owner, &recipient).is_ok());
        recipient.membership.active = false;
        assert_eq!(
            policy.authorize_share(&owner, &recipient),
            Err(PolicyError::InactiveMembership)
        );
    }
}
