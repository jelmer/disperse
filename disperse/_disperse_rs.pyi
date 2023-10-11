
def check_new_revisions(branch, news_file: str | None) -> bool: ...

def increase_version(version: str, level: int = -1) -> str: ...

def expand_tag(template: str, version: str) -> str: ...

def unexpand_tag(template: str, tag: str) -> str: ...

def get_owned_crates(user: str) -> list[str]: ...

def cargo_publish(tree, subpath: str | None = None) -> None: ...

def find_version_in_cargo(tree) -> str: ...
def find_version_in_pyproject_toml(tree) -> str: ...

def find_last_version_in_tags(branch, tag_name) -> tuple[str, str | None]: ...

def update_version_in_cargo(tree, version: str) -> None: ...

def update_version_in_manpage(tree, path, new_version: str, release_date) -> None: ...

def update_version_in_pyproject_toml(tree, new_version: str) -> None: ...

def pypi_discover_urls(username: str) -> list[str]: ...

def find_name_in_pyproject_toml(tree) -> str: ...

def pyproject_uses_hatch_vcs(tree) -> bool: ...

def find_hatch_vcs_version(tree) -> str: ...

def read_project_urls_from_pyproject_toml(path) -> list[tuple[str, str | None]]: ...
def read_project_urls_from_setup_cfg(path) -> list[tuple[str, str | None]]: ...

def upload_python_artifacts(local_tree, pypi_paths) -> None: ...

class UploadCommandFailed(Exception): ...

def create_setup_py_artifacts(tree) -> list[str]: ...

def create_python_artifacts(tree) -> list[str]: ...
