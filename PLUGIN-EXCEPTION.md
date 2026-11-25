# homeserver Plugin Exception

Version 1.0

Additional permission under section 7 of the GNU Affero General Public License, version 3 ("AGPLv3").

This Additional Permission applies to homeserver (the "Program") as distributed by its copyright holders, in addition to the terms of the AGPLv3.

## 0. Definitions

"Plugin Interface" means the interface through which the Program loads and communicates with Plugins, as documented and published by the Program's maintainers. It consists of (a) the WebAssembly host functions, imports, exports and data formats that the Program makes available to Plugins; (b) the user-interface extension API that the Program makes available to user-interface extensions; and (c) the software development kits for these interfaces that are distributed under permissive licenses, including the `hs-plugin-api` crate.

"Plugin" means a work that is designed to be loaded by the Program, that interacts with the Program exclusively through the Plugin Interface (for example, a WebAssembly module executed in the Program's plugin sandbox, or a user-interface extension running in the Program's sandboxed extension frame), and that does not incorporate any part of the Program other than material from the Plugin Interface that is licensed under permissive terms.

## 1. Grant of Additional Permission

You have permission to create, use, copy, modify and convey a Plugin under terms of your choice, and to load it into and run it with the Program or a work based on the Program. The Plugin is not considered a work based on the Program and is not subject to the terms of the AGPLv3, even if the Plugin and the Program together are regarded as a single combined work.

## 2. No Effect on the Program

This Additional Permission does not change the terms that apply to the Program itself. The Program, including any modified version of it, remains subject to the AGPLv3 in full, including section 13 (Remote Network Interaction; Use with the GNU General Public License). A work that modifies the Program, or that interacts with the Program other than through the Plugin Interface, is not a Plugin.

## 3. Modified Versions

If you modify the Program, you may extend this Additional Permission to your version of the Program, but you are not obligated to do so. If you do not wish to do so, delete this Additional Permission from your version, as permitted by section 7 of the AGPLv3.
