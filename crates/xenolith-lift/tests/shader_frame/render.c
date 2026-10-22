/* Draws one triangle with a given vertex module and reads the pixels back.
 *
 * The vertex module reads its position from a storage buffer of vertices, and
 * its constants from a uniform block, so both are made here and filled the way
 * the runtime fills them. What the frame holds says whether the shader put
 * anything on the surface, which no amount of reading the module can.
 */
#include <vulkan/vulkan.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define W 64
#define H 64
#define CONSTANTS 512
#define VERTEX_WORDS 256

static unsigned char *slurp(const char *p, size_t *n) {
    FILE *f = fopen(p, "rb");
    if (!f) { printf("no %s\n", p); exit(1); }
    fseek(f, 0, SEEK_END); *n = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char *b = malloc(*n);
    if (fread(b, 1, *n, f) != *n) { }
    fclose(f);
    return b;
}

static uint32_t pick(uint32_t type_bits, VkPhysicalDevice dev0, VkDeviceSize need) {
    VkPhysicalDeviceMemoryProperties mp;
    vkGetPhysicalDeviceMemoryProperties(dev0, &mp);
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++) {
        if ((mp.memoryTypes[i].propertyFlags & type_bits) == type_bits) return i;
    }
    return 0;
}

int main(void) {
    size_t vn, pn;
    unsigned char *vs = slurp("vert.spv", &vn);
    unsigned char *ps = slurp("frag.spv", &pn);

    VkApplicationInfo app = {VK_STRUCTURE_TYPE_APPLICATION_INFO};
    app.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo ici = {VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    ici.pApplicationInfo = &app;
    VkInstance inst; vkCreateInstance(&ici, 0, &inst);
    uint32_t n = 0; vkEnumeratePhysicalDevices(inst, &n, 0);
    VkPhysicalDevice dev0; vkEnumeratePhysicalDevices(inst, &n, &dev0);

    float pri = 1.0f;
    VkDeviceQueueCreateInfo qci = {VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
    qci.queueCount = 1; qci.pQueuePriorities = &pri;
    VkPhysicalDeviceFeatures feats = {0};
    feats.robustBufferAccess = VK_TRUE;
    VkDeviceCreateInfo dci = {VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    dci.queueCreateInfoCount = 1; dci.pQueueCreateInfos = &qci; dci.pEnabledFeatures = &feats;
    VkDevice dev; vkCreateDevice(dev0, &dci, 0, &dev);
    VkQueue q; vkGetDeviceQueue(dev, 0, 0, &q);

    /* The uniform block: 514 vec4s, the first two of which are the scale and
     * the offset the shader multiplies its position by. */
    VkDeviceSize const_size = (VkDeviceSize)(CONSTANTS + 2) * 16u;
    VkBufferCreateInfo bci = {VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
    bci.size = const_size; bci.usage = VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
    VkBuffer cbuf; vkCreateBuffer(dev, &bci, 0, &cbuf);
    VkMemoryRequirements cr; vkGetBufferMemoryRequirements(dev, cbuf, &cr);
    VkMemoryAllocateInfo cai = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    cai.allocationSize = cr.size;
    cai.memoryTypeIndex = pick(VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT, dev0, cr.size);
    VkDeviceMemory cmem; vkAllocateMemory(dev, &cai, 0, &cmem);
    vkBindBufferMemory(dev, cbuf, cmem, 0);
    void *cp; vkMapMemory(dev, cmem, 0, VK_WHOLE_SIZE, 0, &cp);
    float *cf = cp;
    for (unsigned i = 0; i < (CONSTANTS + 2) * 4u; i++) cf[i] = 0.0f;
    /* Scale and offset, as the runtime would set them. */
    /* The scale and offset that bring a pixel position into the range a host
     * takes, which is what a shader multiplying by these needs. */
    cf[0] = 1.0f; cf[1] = 1.0f; cf[2] = 1.0f; cf[3] = 1.0f;
    cf[4] = 0.0f; cf[5] = 0.0f; cf[6] = 0.0f; cf[7] = 1.0f;

    /* The vertices: three corners of a triangle in clip space. */
    VkDeviceSize vert_size = VERTEX_WORDS * 4u;
    bci.size = vert_size; bci.usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
    VkBuffer vbuf; vkCreateBuffer(dev, &bci, 0, &vbuf);
    vkGetBufferMemoryRequirements(dev, vbuf, &cr);
    cai.allocationSize = cr.size;
    cai.memoryTypeIndex = pick(VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT, dev0, cr.size);
    VkDeviceMemory vmem; vkAllocateMemory(dev, &cai, 0, &vmem);
    vkBindBufferMemory(dev, vbuf, vmem, 0);
    void *vp; vkMapMemory(dev, vmem, 0, VK_WHOLE_SIZE, 0, &vp);
    float *vf = vp;
    for (unsigned i = 0; i < VERTEX_WORDS; i++) vf[i] = 0.0f;
    /* Three corners at the front of the buffer, two floats each, as the
     * streams the module declares expect. */
    /* The window is an array of words, and its first words are where each
     * stream begins. Word 0 is the slot the fetch reads its base from, so the
     * corners go after that, one per vertex, seven words each. */
    unsigned base = 1;
    ((uint32_t *)vp)[0] = base;
    float *corner = (float *)&((uint32_t *)vp)[base];
    /* Three corners of a triangle, each four words of position then three more
     * the streams name. */
    float corners[3][4] = {{-1.0f, 3.0f, 0.0f, 1.0f},
                           {-1.0f, -1.0f, 0.0f, 1.0f},
                           {3.0f, -1.0f, 0.0f, 1.0f}};
    for (unsigned c = 0; c < 3; c++) {
        for (unsigned k = 0; k < 4; k++) corner[c * 7 + k] = corners[c][k];
        for (unsigned k = 4; k < 7; k++) corner[c * 7 + k] = 0.0f;
    }

    /* The descriptor set: constants at 0, vertices at 1. */
    VkDescriptorSetLayoutBinding binds[2] = {{0}, {0}};
    binds[0].binding = 0; binds[0].descriptorType = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
    binds[0].descriptorCount = 1;
    binds[0].stageFlags = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;
    binds[1].binding = 1; binds[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    binds[1].descriptorCount = 1;
    binds[1].stageFlags = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;
    VkDescriptorSetLayoutCreateInfo dl = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
    dl.bindingCount = 2; dl.pBindings = binds;
    VkDescriptorSetLayout dsl; vkCreateDescriptorSetLayout(dev, &dl, 0, &dsl);

    VkDescriptorPoolSize sizes[2] = {{VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1},
                                     {VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1}};
    VkDescriptorPoolCreateInfo dpi = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
    dpi.maxSets = 1; dpi.poolSizeCount = 2; dpi.pPoolSizes = sizes;
    VkDescriptorPool pool; vkCreateDescriptorPool(dev, &dpi, 0, &pool);
    VkDescriptorSetAllocateInfo dai = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
    dai.descriptorPool = pool; dai.descriptorSetCount = 1; dai.pSetLayouts = &dsl;
    VkDescriptorSet set; vkAllocateDescriptorSets(dev, &dai, &set);

    VkDescriptorBufferInfo ci = {cbuf, 0, VK_WHOLE_SIZE};
    VkDescriptorBufferInfo vi = {vbuf, 0, VK_WHOLE_SIZE};
    VkWriteDescriptorSet w[2] = {{VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}, {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}};
    w[0].dstSet = set; w[0].dstBinding = 0; w[0].descriptorCount = 1;
    w[0].descriptorType = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER; w[0].pBufferInfo = &ci;
    w[1].dstSet = set; w[1].dstBinding = 1; w[1].descriptorCount = 1;
    w[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER; w[1].pBufferInfo = &vi;
    vkUpdateDescriptorSets(dev, 2, w, 0, NULL);

    VkPipelineLayoutCreateInfo pl = {VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
    pl.setLayoutCount = 1; pl.pSetLayouts = &dsl;
    VkPipelineLayout layout; vkCreatePipelineLayout(dev, &pl, 0, &layout);

    /* A colour target to draw into and read back. */
    VkImageCreateInfo ici2 = {VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
    ici2.imageType = VK_IMAGE_TYPE_2D; ici2.format = VK_FORMAT_R8G8B8A8_UNORM;
    ici2.extent.width = W; ici2.extent.height = H; ici2.extent.depth = 1;
    ici2.mipLevels = 1; ici2.arrayLayers = 1; ici2.samples = VK_SAMPLE_COUNT_1_BIT;
    ici2.tiling = VK_IMAGE_TILING_OPTIMAL;
    ici2.usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
    VkImage img; vkCreateImage(dev, &ici2, 0, &img);
    VkMemoryRequirements ir; vkGetImageMemoryRequirements(dev, img, &ir);
    VkMemoryAllocateInfo iai = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    iai.allocationSize = ir.size;
    iai.memoryTypeIndex = pick(VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, dev0, ir.size);
    VkDeviceMemory imem; vkAllocateMemory(dev, &iai, 0, &imem);
    vkBindImageMemory(dev, img, imem, 0);
    VkImageViewCreateInfo iv = {VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
    iv.image = img; iv.viewType = VK_IMAGE_VIEW_TYPE_2D; iv.format = VK_FORMAT_R8G8B8A8_UNORM;
    iv.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    iv.subresourceRange.levelCount = 1; iv.subresourceRange.layerCount = 1;
    VkImageView view; vkCreateImageView(dev, &iv, 0, &view);

    VkAttachmentDescription at = {0};
    at.format = VK_FORMAT_R8G8B8A8_UNORM; at.samples = VK_SAMPLE_COUNT_1_BIT;
    at.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR; at.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    at.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    at.finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    VkAttachmentReference ref = {0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription sp = {0};
    sp.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    sp.colorAttachmentCount = 1; sp.pColorAttachments = &ref;
    VkRenderPassCreateInfo rp = {VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
    rp.attachmentCount = 1; rp.pAttachments = &at;
    rp.subpassCount = 1; rp.pSubpasses = &sp;
    VkRenderPass pass; vkCreateRenderPass(dev, &rp, 0, &pass);
    VkFramebufferCreateInfo fbi = {VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
    fbi.renderPass = pass; fbi.attachmentCount = 1; fbi.pAttachments = &view;
    fbi.width = W; fbi.height = H; fbi.layers = 1;
    VkFramebuffer fb; vkCreateFramebuffer(dev, &fbi, 0, &fb);

    VkShaderModuleCreateInfo sm = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
    sm.codeSize = vn; sm.pCode = (const uint32_t *)vs;
    VkShaderModule mv; vkCreateShaderModule(dev, &sm, 0, &mv);
    sm.codeSize = pn; sm.pCode = (const uint32_t *)ps;
    VkShaderModule mf; vkCreateShaderModule(dev, &sm, 0, &mf);

    VkPipelineShaderStageCreateInfo st[2] = {{VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO},
                                             {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO}};
    st[0].stage = VK_SHADER_STAGE_VERTEX_BIT; st[0].module = mv; st[0].pName = "main";
    st[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT; st[1].module = mf; st[1].pName = "main";
    VkPipelineVertexInputStateCreateInfo vin = {VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
    VkPipelineInputAssemblyStateCreateInfo ia = {VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO};
    ia.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    VkViewport vpp = {0, 0, W, H, 0, 1};
    VkRect2D sc = {{0, 0}, {W, H}};
    VkPipelineViewportStateCreateInfo vps = {VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO};
    vps.viewportCount = 1; vps.pViewports = &vpp; vps.scissorCount = 1; vps.pScissors = &sc;
    VkPipelineRasterizationStateCreateInfo rs = {VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO};
    rs.polygonMode = VK_POLYGON_MODE_FILL; rs.cullMode = VK_CULL_MODE_NONE;
    rs.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE; rs.lineWidth = 1.0f;
    VkPipelineMultisampleStateCreateInfo ms = {VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO};
    ms.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
    VkPipelineColorBlendAttachmentState ba = {0}; ba.colorWriteMask = 0xf;
    VkPipelineColorBlendStateCreateInfo cb = {VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO};
    cb.attachmentCount = 1; cb.pAttachments = &ba;
    VkGraphicsPipelineCreateInfo gp = {VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO};
    gp.stageCount = 2; gp.pStages = st; gp.pVertexInputState = &vin;
    gp.pInputAssemblyState = &ia; gp.pViewportState = &vps;
    gp.pRasterizationState = &rs; gp.pMultisampleState = &ms;
    gp.pColorBlendState = &cb; gp.layout = layout; gp.renderPass = pass;
    VkPipeline pipe;
    printf("pipeline: %d\n", vkCreateGraphicsPipelines(dev, VK_NULL_HANDLE, 1, &gp, 0, &pipe));

    VkCommandPoolCreateInfo cpi = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
    cpi.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
    VkCommandPool pool2; vkCreateCommandPool(dev, &cpi, 0, &pool2);
    VkCommandBufferAllocateInfo cbi = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
    cbi.commandPool = pool2; cbi.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY; cbi.commandBufferCount = 1;
    VkCommandBuffer cmd; vkAllocateCommandBuffers(dev, &cbi, &cmd);
    VkCommandBufferBeginInfo bi = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    vkBeginCommandBuffer(cmd, &bi);
    VkClearValue clear = {0};
    VkRenderPassBeginInfo rbi = {VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
    rbi.renderPass = pass; rbi.framebuffer = fb;
    rbi.renderArea.extent.width = W; rbi.renderArea.extent.height = H;
    rbi.clearValueCount = 1; rbi.pClearValues = &clear;
    vkCmdBeginRenderPass(cmd, &rbi, VK_SUBPASS_CONTENTS_INLINE);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, pipe);
    vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, layout, 0, 1, &set, 0, NULL);
    vkCmdDraw(cmd, 6, 1, 0, 0);
    vkCmdEndRenderPass(cmd);
    vkEndCommandBuffer(cmd);
    VkSubmitInfo si = {VK_STRUCTURE_TYPE_SUBMIT_INFO};
    si.commandBufferCount = 1; si.pCommandBuffers = &cmd;
    vkQueueSubmit(q, 1, &si, 0); vkQueueWaitIdle(q);

    /* Read it back. */
    bci.size = W * H * 4; bci.usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT;
    VkBuffer rbuf; vkCreateBuffer(dev, &bci, 0, &rbuf);
    vkGetBufferMemoryRequirements(dev, rbuf, &cr);
    cai.allocationSize = cr.size;
    cai.memoryTypeIndex = pick(VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT, dev0, cr.size);
    VkDeviceMemory rmem; vkAllocateMemory(dev, &cai, 0, &rmem);
    vkBindBufferMemory(dev, rbuf, rmem, 0);
    vkResetCommandBuffer(cmd, 0); vkBeginCommandBuffer(cmd, &bi);
    VkBufferImageCopy bic = {0};
    bic.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    bic.imageSubresource.layerCount = 1;
    bic.imageExtent.width = W; bic.imageExtent.height = H; bic.imageExtent.depth = 1;
    vkCmdCopyImageToBuffer(cmd, img, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, rbuf, 1, &bic);
    vkEndCommandBuffer(cmd);
    vkQueueSubmit(q, 1, &si, 0); vkQueueWaitIdle(q);
    void *rp2; vkMapMemory(dev, rmem, 0, VK_WHOLE_SIZE, 0, &rp2);
    unsigned char *px = rp2;
    unsigned lit = 0;
    for (unsigned i = 0; i < W * H; i++) {
        if (px[i * 4] || px[i * 4 + 1] || px[i * 4 + 2]) lit++;
    }
    printf("lit pixels: %u of %u\n", lit, W * H);
    /* The buffer's first words, which is what the fetch reads as its base. */
    printf("buffer word 0 (base) = %u\n", ((uint32_t *)vp)[0]);
    printf("corner 0 = %g %g %g %g\n", corner[0], corner[1], corner[2], corner[3]);
    printf("centre: %u %u %u\n", px[(H / 2 * W + W / 2) * 4], px[(H / 2 * W + W / 2) * 4 + 1],
           px[(H / 2 * W + W / 2) * 4 + 2]);
    return 0;
}
