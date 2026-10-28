
## [0.1.1] - 2026-10-28

### Features

- *(analysis)* Name every origin the report counts - ([9ab9cee](https://github.com/QaidVoid/xenolith/commit/9ab9cee6d6056072df44a454841156ab0c4d2c62))
- *(analysis)* Find the functions in a hole or at a handover - ([613e6e5](https://github.com/QaidVoid/xenolith/commit/613e6e5c769b334b05cfaac97ece849e7db6418e))
- *(analysis)* Find the functions a record or a table names - ([f09fdb2](https://github.com/QaidVoid/xenolith/commit/f09fdb2e97737b5b412b9de4d56b7255019aedc7))
- *(cli)* Report imports apart from what failed to lift - ([7a33b10](https://github.com/QaidVoid/xenolith/commit/7a33b1046fa406d9d70f72ce35bd5c487c0932b8))
- *(cli)* List the imports a container declares - ([8f6c426](https://github.com/QaidVoid/xenolith/commit/8f6c426b8c812051aedd29d05d0b66bc60701e43))
- *(cli)* Add lift subcommand - ([ddbb8d1](https://github.com/QaidVoid/xenolith/commit/ddbb8d103daa39ec94ee707bab3eb0964fa30b08))
- *(cli)* Report resolved and unresolved branches apart - ([0699602](https://github.com/QaidVoid/xenolith/commit/069960235fa6ef0a2fb9448a36c5d5ac9433b72a))
- *(cli)* Add analyze subcommand - ([2399f19](https://github.com/QaidVoid/xenolith/commit/2399f19f9fb6a4d7f3e299ad9c41133a9caf1fa4))
- *(cli)* Add disassemble subcommand with coverage sweep - ([a74d803](https://github.com/QaidVoid/xenolith/commit/a74d803e81874f210f17d3d8d065e15825b633f5))
- *(cli)* Add inspect subcommand for xex files - ([a279819](https://github.com/QaidVoid/xenolith/commit/a279819a672559760e713a1036bc7da15660f7a3))
- *(lift)* Name a game's program and link its files - ([c198cc3](https://github.com/QaidVoid/xenolith/commit/c198cc32333bf951f025e2ba239d7fae2d4467b5))
- *(lift)* Give the kernel variables a title imports a home - ([65edfb2](https://github.com/QaidVoid/xenolith/commit/65edfb2dfc7cf533e055be202fe34c12fe2fe434))
- *(lift)* Put the shader library beside the code - ([cc9acfe](https://github.com/QaidVoid/xenolith/commit/cc9acfef2f7d2a0aa79c15050080c99713931844))
- *(lift)* Emit the shaders an image holds - ([d9fb310](https://github.com/QaidVoid/xenolith/commit/d9fb310512ceb0921280fd8dd4a882b7cd276a9c))
- *(lift)* Tell the boot program what kernel build it has - ([1fd0011](https://github.com/QaidVoid/xenolith/commit/1fd00115e26927a9ad916bc271542d2370ebfb3f))
- *(lift)* Take an address a program reached as a root - ([c3ea0e3](https://github.com/QaidVoid/xenolith/commit/c3ea0e39ca6269e0abb89eef2c5a45f36a6fb82a))
- *(lift)* Name an import where one is reported - ([861668a](https://github.com/QaidVoid/xenolith/commit/861668ab6f2b146cbc962fa243bcf26df069f1b8))
- *(lift)* Ship a runtime the emitted code links against - ([9c91c57](https://github.com/QaidVoid/xenolith/commit/9c91c572864bd8b38201dcf4bf599c0037d16ad9))
- *(lift)* Model the console vector forms - ([f43e7ee](https://github.com/QaidVoid/xenolith/commit/f43e7ee3a0e572d7bf6d3642ab23ad21deedcab7))
- *(lift)* Model the vector lane operations - ([b2a9c96](https://github.com/QaidVoid/xenolith/commit/b2a9c9692d1a690c2def32fc0fe7c4dd2eeb4b77))
- *(lift)* Emit an import thunk as a call - ([11226d7](https://github.com/QaidVoid/xenolith/commit/11226d720b58fe0d4f6420e943b77555086478b2))
- *(lift)* Split the emitted C into translation units - ([33e274c](https://github.com/QaidVoid/xenolith/commit/33e274cb5df0d4f58394ed3d1581bad4c87bcfba))
- *(runtime)* Answer a title's privilege and display settings - ([7ba4430](https://github.com/QaidVoid/xenolith/commit/7ba443051ba646405335e6ce7e9a247016cbd70a))
- *(runtime)* Draw what a title asks for - ([b12506b](https://github.com/QaidVoid/xenolith/commit/b12506bbb603242ff77cee90d812809f6c9b7f73))
- *(runtime)* Bring the renderer into the build - ([043d5dd](https://github.com/QaidVoid/xenolith/commit/043d5dd9a086f1312d80935ed7dbb768340bfe28))
- *(runtime)* Play the sound a title mixes - ([45324cc](https://github.com/QaidVoid/xenolith/commit/45324cc031a49f182dadf6cbfedf071e72b2e285))
- *(runtime)* Read the commands a title writes - ([e26bb39](https://github.com/QaidVoid/xenolith/commit/e26bb39e6975ca74e2ce39f0670d162e847581b6))
- *(runtime)* Run a title's threads on real threads - ([92a6566](https://github.com/QaidVoid/xenolith/commit/92a6566cb1ca2252f5e781813e6c98797c3f09c1))
- *(runtime)* Tell an unlifted function from a trap - ([2e69667](https://github.com/QaidVoid/xenolith/commit/2e69667f4b9bb352fbc1f40be21161b141845741))
- *(runtime)* Collect every address a run wanted - ([af945a2](https://github.com/QaidVoid/xenolith/commit/af945a280c66210a086759775c737652636d2618))
- *(runtime)* Give a title a stack and its startup calls - ([f7fe416](https://github.com/QaidVoid/xenolith/commit/f7fe4166cc510a6b4f5ea51b03f9fe4db6b60242))
- *(xex)* Distinguish unbuilt from unrecognized compression - ([4090e87](https://github.com/QaidVoid/xenolith/commit/4090e87d73ceaaa457d5261f7e0ea0d668faff24))
- *(xex)* Add workspace and container header parsing - ([f96ac84](https://github.com/QaidVoid/xenolith/commit/f96ac84beabf25509627bc64ff31f7b882b4a4db))
- Boot a built game from the player's own copy - ([d0cd9c9](https://github.com/QaidVoid/xenolith/commit/d0cd9c9461f964a1077121cf94dc9c4a93fd0fe5))

### Bug Fixes

- *(cli)* Use the key file a caller named - ([157a145](https://github.com/QaidVoid/xenolith/commit/157a145f1809cd843314b89197f2a4a427c5958d))
- *(lift)* Rebuild the runtime when a file it includes changes - ([612996b](https://github.com/QaidVoid/xenolith/commit/612996b87b84f630ab688029076a488a565917f8))
- *(lift)* Stop claiming a program stops at an import - ([aaa2ab5](https://github.com/QaidVoid/xenolith/commit/aaa2ab55fbf2996909a08abee8d515ed66c09ced))
- *(lift)* Emit a body for every helper entry the output names - ([088a8d4](https://github.com/QaidVoid/xenolith/commit/088a8d4d693e397dac40a0ab90f86fd07dc4c553))
- *(lift)* Emit where control continues past a block - ([15ec4bc](https://github.com/QaidVoid/xenolith/commit/15ec4bc4d56866a24569997ed8df3c027ab36f12))
- *(runtime)* Draw by a title's own vertex numbers - ([81bb1f0](https://github.com/QaidVoid/xenolith/commit/81bb1f0a99ed87c3a9d84382c52e20a4e2dde806))

### Documentation

- *(output)* List every file a lift writes, and check it - ([0008470](https://github.com/QaidVoid/xenolith/commit/00084704d916f42fe3f31e5608a0f5faac42133f))
- Make every quoted number come from the tool - ([48abefa](https://github.com/QaidVoid/xenolith/commit/48abefa0c344415739f90ef9c45c50b17abf8a9f))
- State every exit code and test it - ([a6f87b0](https://github.com/QaidVoid/xenolith/commit/a6f87b0de20a38f1cc3a4a2f3c49d7ef88ef6268))
- Drop inline comments that restate or narrate - ([b664074](https://github.com/QaidVoid/xenolith/commit/b664074e41a8439fa18b26b15f8f70e91d802cd6))

### Testing

- *(cli)* Cover the flags a caller depends on - ([5cce443](https://github.com/QaidVoid/xenolith/commit/5cce44358eca8ce5c64d1665ef608cbf980c5443))
- *(lift)* Link the emitted code with and without graphics - ([547c5de](https://github.com/QaidVoid/xenolith/commit/547c5dec43074229c42b6f00b7387417d490c1c6))

### Miscellaneous Tasks

- Release the tool on GitHub with release-plz - ([2a39552](https://github.com/QaidVoid/xenolith/commit/2a395528a6f8a84253052ebca3e325340914af3e))
