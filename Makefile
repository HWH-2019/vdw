.PHONY: package package-vdw package-vdw-dmg package-vscode

package:
	bash scripts/package_all.sh

package-vdw:
	bash scripts/package_vdw.sh darwin arm64

package-vdw-dmg:
	bash scripts/package_vdw_dmg.sh arm64

package-vscode:
	bash scripts/package_vscode.sh
